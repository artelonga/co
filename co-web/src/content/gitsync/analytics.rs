//! CO-89: per-profile analytics computed from `commit` entries.
//!
//! No separate analytics store — everything is a fold over the universe's
//! `commit` entries (the same data the changelog view renders). `now_ns` is
//! threaded in (never read from the system clock) so the computation is fully
//! deterministic and unit-testable.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::domain::EntryDomain;
use crate::repository::EntryStore;

/// 30 days in nanoseconds — the window for `commits_last_30d`.
const WINDOW_30D_NS: i64 = 30 * 86_400 * 1_000_000_000;

fn fm_i64(e: &EntryDomain, key: &str) -> i64 {
    e.frontmatter.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn fm_str<'a>(e: &'a EntryDomain, key: &str) -> Option<&'a str> {
    e.frontmatter.get(key).and_then(Value::as_str)
}

/// Compute the analytics struct for a set of commit entries (already filtered to
/// one author). `now_ns` is the reference "now" for the rolling-30-day window.
pub fn compute_analytics(commits: &[EntryDomain], now_ns: i64) -> Value {
    if commits.is_empty() {
        return json!({
            "commits_total": 0,
            "commits_last_30d": 0,
            "lines_added_total": 0,
            "lines_removed_total": 0,
            "first_commit_at": Value::Null,
            "last_commit_at": Value::Null,
            "frequent_scopes": [],
            "mean_commit_size_lines": 0,
        });
    }

    let mut lines_added = 0i64;
    let mut lines_removed = 0i64;
    let mut last_30d = 0i64;
    let mut first_ns = i64::MAX;
    let mut last_ns = i64::MIN;
    let mut first_at: Option<String> = None;
    let mut last_at: Option<String> = None;
    let mut scope_counts: BTreeMap<String, i64> = BTreeMap::new();

    for c in commits {
        lines_added += fm_i64(c, "insertions");
        lines_removed += fm_i64(c, "deletions");

        let ts = fm_i64(c, "committed_at_ns");
        if ts >= now_ns - WINDOW_30D_NS && ts <= now_ns {
            last_30d += 1;
        }
        if ts < first_ns {
            first_ns = ts;
            first_at = fm_str(c, "committed_at").map(str::to_string);
        }
        if ts > last_ns {
            last_ns = ts;
            last_at = fm_str(c, "committed_at").map(str::to_string);
        }

        if let Some(tags) = c.frontmatter.get("tags").and_then(Value::as_array) {
            for tag in tags.iter().filter_map(Value::as_str) {
                *scope_counts.entry(tag.to_string()).or_insert(0) += 1;
            }
        }
    }

    let total = commits.len() as i64;
    // Top scopes by count desc, then name asc for stable ordering.
    let mut scopes: Vec<(String, i64)> = scope_counts.into_iter().collect();
    scopes.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let frequent_scopes: Vec<Value> = scopes
        .into_iter()
        .take(10)
        .map(|(name, n)| json!([name, n]))
        .collect();

    let mean = if total > 0 {
        (lines_added + lines_removed) / total
    } else {
        0
    };

    json!({
        "commits_total": total,
        "commits_last_30d": last_30d,
        "lines_added_total": lines_added,
        "lines_removed_total": lines_removed,
        "first_commit_at": first_at,
        "last_commit_at": last_at,
        "frequent_scopes": frequent_scopes,
        "mean_commit_size_lines": mean,
    })
}

/// Recompute and persist the `analytics` struct on `profiles/<handle>.md`.
/// Returns the freshly-computed struct. No-op (returns `Ok(None)`) if the
/// profile entry doesn't exist.
pub fn recompute_profile_analytics(
    store: &dyn EntryStore,
    handle: &str,
    now_ns: i64,
) -> anyhow::Result<Option<Value>> {
    let path = crate::content::gitsync::profiles::profile_path(handle);
    let Some(mut profile) = store.get(&path)? else {
        return Ok(None);
    };

    let commits = store.list("commit", &json!({}), None)?;
    let mine: Vec<EntryDomain> = commits
        .into_iter()
        .filter(|c| fm_str(c, "author_handle") == Some(handle))
        .collect();

    let analytics = compute_analytics(&mine, now_ns);
    if let Some(obj) = profile.frontmatter.as_object_mut() {
        obj.insert("analytics".to_string(), analytics.clone());
    }
    profile.body_hash = co::entry::Entry::hash_body(&profile.body);
    store.upsert(&profile)?;
    Ok(Some(analytics))
}

/// Recompute analytics for every profile entry in the universe. Returns the
/// number of profiles updated. Used by the nightly job + the admin endpoint.
pub fn recompute_all_profiles(store: &dyn EntryStore, now_ns: i64) -> anyhow::Result<usize> {
    let profiles = store.list("profile", &json!({}), None)?;
    let mut updated = 0;
    for profile in profiles {
        if let Some(handle) = fm_str(&profile, "handle")
            && recompute_profile_analytics(store, handle, now_ns)?.is_some()
        {
            updated += 1;
        }
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::gitsync::git_log::parse_git_log;
    use crate::content::gitsync::git_log::{RS, US};
    use crate::content::gitsync::ingest::ingest_commits;
    use crate::universe_pool::UniversePool;
    use tempfile::tempdir;

    // now = 2026-04-27T00:00:00Z in ns.
    fn now_ns() -> i64 {
        chrono::DateTime::parse_from_rfc3339("2026-04-27T00:00:00Z")
            .unwrap()
            .timestamp_nanos_opt()
            .unwrap()
    }

    fn log() -> String {
        // yuri: 2 commits (one recent, one ancient); claude: 1.
        format!(
            "{RS}s1{US}2026-04-26T12:00:00-03:00{US}Yuri{US}yuri@artelonga.com.br\
{US}Yuri{US}yuri@artelonga.com.br{US}{US}feat(co-web): CO-65 a{US}\n 2 files changed, 10 insertions(+), 4 deletions(-)\n\
{RS}s2{US}2026-04-20T12:00:00-03:00{US}Yuri{US}yuri@artelonga.com.br\
{US}Yuri{US}yuri@artelonga.com.br{US}{US}fix(co-web): CO-66 b{US}\n 1 file changed, 6 insertions(+)\n\
{RS}s3{US}2025-01-01T12:00:00-03:00{US}Claude{US}noreply@anthropic.com\
{US}Claude{US}noreply@anthropic.com{US}{US}chore(roadmap): c{US}\n 1 file changed, 2 insertions(+), 1 deletion(-)\n"
        )
    }

    #[test]
    fn computes_real_numbers_for_yuri() {
        let dir = tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);
        let store = pool.entry_store("co-dev").unwrap();
        let recs = parse_git_log(&log());
        ingest_commits(store.as_ref(), "co-dev", &recs, None).unwrap();

        let a = recompute_profile_analytics(store.as_ref(), "yuri", now_ns())
            .unwrap()
            .unwrap();
        assert_eq!(a["commits_total"], 2);
        assert_eq!(a["commits_last_30d"], 2); // both within 30d of 2026-04-27
        assert_eq!(a["lines_added_total"], 16); // 10 + 6
        assert_eq!(a["lines_removed_total"], 4);
        assert_eq!(a["mean_commit_size_lines"], 10); // (16+4)/2
        assert_eq!(a["first_commit_at"], "2026-04-20T12:00:00-03:00");
        assert_eq!(a["last_commit_at"], "2026-04-26T12:00:00-03:00");
        assert_eq!(a["frequent_scopes"][0][0], "co-web");
        assert_eq!(a["frequent_scopes"][0][1], 2);

        // Persisted onto the profile entry.
        let profile = store.get("profiles/yuri.md").unwrap().unwrap();
        assert_eq!(profile.frontmatter["analytics"]["commits_total"], 2);
    }

    #[test]
    fn ancient_commit_excluded_from_30d_window() {
        let dir = tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);
        let store = pool.entry_store("co-dev").unwrap();
        let recs = parse_git_log(&log());
        ingest_commits(store.as_ref(), "co-dev", &recs, None).unwrap();

        let a = recompute_profile_analytics(store.as_ref(), "claude", now_ns())
            .unwrap()
            .unwrap();
        assert_eq!(a["commits_total"], 1);
        assert_eq!(a["commits_last_30d"], 0); // 2025-01-01 is far outside 30d
    }

    #[test]
    fn recompute_all_covers_every_profile() {
        let dir = tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);
        let store = pool.entry_store("co-dev").unwrap();
        let recs = parse_git_log(&log());
        ingest_commits(store.as_ref(), "co-dev", &recs, None).unwrap();

        let n = recompute_all_profiles(store.as_ref(), now_ns()).unwrap();
        assert_eq!(n, 2);
    }
}
