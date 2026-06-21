//! CO-89: turn parsed [`CommitRecord`]s into content entries and write them
//! through a per-universe [`EntryStore`].
//!
//! Pure mapping (`commit_to_entry`) is separated from the write loop
//! (`ingest_commits`) so the mapping is unit-testable and the write loop is
//! covered by an in-process `UniversePool` test (no git, no network).

use serde_json::json;

use crate::content::gitsync::git_log::CommitRecord;
use crate::content::gitsync::profiles::build_profile_entries;
use crate::domain::EntryDomain;
use crate::repository::EntryStore;

/// Vault path for a commit entry — one file per SHA.
pub fn commit_path(sha: &str) -> String {
    format!("commits/{sha}.md")
}

/// Map one commit to a `commit` content entry. The body is the full commit
/// message (so `[[CO-65]]`-style wikilinks in the message resolve via the
/// relation indexer); every structured field lives in frontmatter for
/// json_extract queries + analytics.
pub fn commit_to_entry(universe_key: &str, rec: &CommitRecord) -> EntryDomain {
    let frontmatter = json!({
        "type": "commit",
        "title": rec.subject,
        "sha": rec.sha,
        "author_handle": rec.author_handle,
        "author_name": rec.author_name,
        "author_email": rec.author_email,
        "committer_handle": rec.committer_handle,
        "committer_email": rec.committer_email,
        "committed_at": rec.committed_at,
        "committed_at_ns": rec.committed_at_ns,
        "timezone": rec.tz_offset,
        "parent_task": rec.parent_task,
        "tags": rec.tags,
        "parents": rec.parents,
        "files_changed": rec.files_changed,
        "insertions": rec.insertions,
        "deletions": rec.deletions,
        "is_merge": rec.parents.len() > 1,
    });
    EntryDomain {
        path: commit_path(&rec.sha),
        universe_key: universe_key.to_string(),
        entry_type: "commit".to_string(),
        title: Some(rec.subject.clone()),
        frontmatter,
        body_hash: co::entry::Entry::hash_body(&rec.message),
        body: rec.message.clone(),
        created_at: Some(rec.committed_at.clone()),
        updated_at: Some(rec.committed_at.clone()),
    }
}

/// Outcome of an ingestion pass.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct SyncStats {
    /// Number of new/updated commit entries written.
    pub commits_ingested: usize,
    /// Number of profile entries written.
    pub profiles_written: usize,
    /// SHA of the newest commit ingested (the new sync cursor), if any.
    pub head_sha: Option<String>,
}

/// Write commit + profile entries for `records` into `store`.
///
/// `records` are expected newest-first (git's default). If `since_sha` is set,
/// ingestion stops at the first commit whose SHA matches it (incremental sync) —
/// everything strictly newer than the cursor is imported, the cursor commit and
/// older are skipped. Already-imported SHAs are upserted idempotently.
pub fn ingest_commits(
    store: &dyn EntryStore,
    universe_key: &str,
    records: &[CommitRecord],
    since_sha: Option<&str>,
) -> anyhow::Result<SyncStats> {
    let mut stats = SyncStats::default();
    let head_sha = records.first().map(|r| r.sha.clone());

    let mut fresh: Vec<&CommitRecord> = Vec::new();
    for rec in records {
        if since_sha.is_some_and(|s| s == rec.sha) {
            break; // reached the last-synced commit; older ones already imported
        }
        fresh.push(rec);
    }

    for rec in &fresh {
        store.upsert(&commit_to_entry(universe_key, rec))?;
        stats.commits_ingested += 1;
    }

    // Profiles are rebuilt from the whole window we have (fresh commits) so a
    // contributor's profile exists even on incremental runs.
    let owned: Vec<CommitRecord> = fresh.iter().map(|r| (*r).clone()).collect();
    for profile in build_profile_entries(universe_key, &owned) {
        store.upsert(&profile)?;
        stats.profiles_written += 1;
    }

    stats.head_sha = head_sha;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::gitsync::git_log::{RS, US, parse_git_log};
    use crate::universe_pool::UniversePool;
    use tempfile::tempdir;

    fn log() -> String {
        format!(
            "{RS}newsha{US}2026-04-26T23:00:00-03:00{US}Yuri{US}yuri@artelonga.com.br\
{US}Yuri{US}yuri@artelonga.com.br{US}{US}feat(co-web): CO-65 thing{US}\n 3 files changed, 10 insertions(+), 2 deletions(-)\n\
{RS}oldsha{US}2026-04-25T10:00:00-03:00{US}Claude{US}noreply@anthropic.com\
{US}Claude{US}noreply@anthropic.com{US}{US}chore: y{US}\n 1 file changed, 1 insertion(+)\n"
        )
    }

    #[test]
    fn commit_entry_carries_structured_frontmatter() {
        let recs = parse_git_log(&log());
        let e = commit_to_entry("co-dev", &recs[0]);
        assert_eq!(e.entry_type, "commit");
        assert_eq!(e.path, "commits/newsha.md");
        assert_eq!(e.frontmatter["sha"], "newsha");
        assert_eq!(e.frontmatter["parent_task"], "CO-65");
        assert_eq!(e.frontmatter["author_handle"], "yuri");
        assert_eq!(e.frontmatter["insertions"], 10);
        assert_eq!(e.frontmatter["tags"][0], "co-web");
    }

    #[test]
    fn full_ingest_writes_commits_and_profiles() {
        let dir = tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);
        let store = pool.entry_store("co-dev").unwrap();

        let recs = parse_git_log(&log());
        let stats = ingest_commits(store.as_ref(), "co-dev", &recs, None).unwrap();

        assert_eq!(stats.commits_ingested, 2);
        assert_eq!(stats.profiles_written, 2);
        assert_eq!(stats.head_sha.as_deref(), Some("newsha"));

        // Entries are readable back.
        assert!(store.get("commits/newsha.md").unwrap().is_some());
        assert!(store.get("commits/oldsha.md").unwrap().is_some());
        assert!(store.get("profiles/yuri.md").unwrap().is_some());
        assert!(store.get("profiles/claude.md").unwrap().is_some());
    }

    #[test]
    fn incremental_ingest_stops_at_cursor() {
        let dir = tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);
        let store = pool.entry_store("co-dev").unwrap();

        let recs = parse_git_log(&log());
        // Cursor at oldsha ⇒ only newsha is fresh.
        let stats = ingest_commits(store.as_ref(), "co-dev", &recs, Some("oldsha")).unwrap();
        assert_eq!(stats.commits_ingested, 1);
        assert!(store.get("commits/newsha.md").unwrap().is_some());
        assert!(store.get("commits/oldsha.md").unwrap().is_none());
    }

    #[test]
    fn reingest_is_idempotent() {
        let dir = tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);
        let store = pool.entry_store("co-dev").unwrap();
        let recs = parse_git_log(&log());

        ingest_commits(store.as_ref(), "co-dev", &recs, None).unwrap();
        let again = ingest_commits(store.as_ref(), "co-dev", &recs, None).unwrap();
        // Re-running upserts the same 2 commits (no duplicates, no error).
        assert_eq!(again.commits_ingested, 2);
        let count = store
            .list("commit", &serde_json::json!({}), None)
            .unwrap()
            .len();
        assert_eq!(count, 2);
    }
}
