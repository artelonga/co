//! CO-89: git-backed universes.
//!
//! Every repo-backed universe (opt-in via the `git_source` column on
//! `universes`, migration v89) gains a uniform git-like history: one `commit`
//! content entry per commit, one `profile` entry per contributor, per-profile
//! analytics, and server-side Mermaid (gantt of in-flight tasks + timeline of
//! recent commits). Markdown-only universes (no `git_source`) are untouched.
//!
//! ## Layout
//! * [`git_log`] — pure parser: `git log` text → [`git_log::CommitRecord`].
//! * [`git_runner`] — side-effecting `git clone/fetch/log` (full history).
//! * [`ingest`] — `CommitRecord` → `commit` entries, written via `EntryStore`.
//! * [`profiles`] — `profile` entries from author identities.
//! * [`analytics`] — per-profile analytics folded over `commit` entries.
//! * [`mermaid`] — gantt + timeline Mermaid source from entries.
//! * [`routes`] — `POST .../git-sync`, `POST .../recompute-analytics`,
//!   `GET .../gantt`, `GET .../commit-timeline`.
//!
//! The pure layers (`git_log`, `ingest`, `profiles`, `analytics`, `mermaid`) are
//! deterministically unit-testable with no git/network; `git_runner` and the
//! end-to-end sync are tested against a local `file://` repo.

pub mod analytics;
pub mod git_log;
pub mod git_runner;
pub mod ingest;
pub mod mermaid;
pub mod profiles;
pub mod routes;
pub mod worker;

use std::path::Path;

use crate::repository::EntryStore;

pub use ingest::SyncStats;

/// Run a full git-sync for one universe end to end:
/// 1. clone/fetch the repo (full history) into `dest`,
/// 2. `git log <branch>` → parse → ingest fresh commits + profiles,
/// 3. recompute per-profile analytics as of `now_ns`.
///
/// `since_sha` is the incremental cursor (`git_last_synced_sha`); pass `None`
/// for a full backfill. Returns the [`SyncStats`] (new head SHA becomes the
/// next cursor).
pub fn sync_universe_git(
    store: &dyn EntryStore,
    universe_key: &str,
    remote: &str,
    branch: &str,
    dest: &Path,
    since_sha: Option<&str>,
    now_ns: i64,
) -> anyhow::Result<SyncStats> {
    git_runner::ensure_repo(remote, branch, dest)?;
    let raw = git_runner::run_git_log(dest, branch)?;
    let records = git_log::parse_git_log(&raw);
    let stats = ingest::ingest_commits(store, universe_key, &records, since_sha)?;
    analytics::recompute_all_profiles(store, now_ns)?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universe_pool::UniversePool;
    use std::process::Command;
    use tempfile::tempdir;

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    fn init_repo(dir: &Path) {
        let g = |args: &[&str]| {
            Command::new("git")
                .current_dir(dir)
                .args(args)
                .env("GIT_AUTHOR_DATE", "2026-04-26T23:00:00-03:00")
                .env("GIT_COMMITTER_DATE", "2026-04-26T23:00:00-03:00")
                .env("GIT_AUTHOR_NAME", "Yuri")
                .env("GIT_AUTHOR_EMAIL", "yuri@artelonga.com.br")
                .env("GIT_COMMITTER_NAME", "Yuri")
                .env("GIT_COMMITTER_EMAIL", "yuri@artelonga.com.br")
                .output()
                .expect("git");
        };
        std::fs::create_dir_all(dir).unwrap();
        g(&["init", "--initial-branch=main"]);
        g(&["config", "user.email", "yuri@artelonga.com.br"]);
        g(&["config", "user.name", "Yuri"]);
        std::fs::write(dir.join("a.txt"), "x\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-m", "feat(co-web): CO-65 first"]);
    }

    #[test]
    fn end_to_end_sync_ingests_and_computes_analytics() {
        if !git_available() {
            return;
        }
        let tmp = tempdir().unwrap();
        let origin = tmp.path().join("origin");
        init_repo(&origin);
        let remote = format!("file://{}", origin.display());

        let pool = UniversePool::new(tmp.path(), 16);
        let store = pool.entry_store("co-dev").unwrap();
        let dest = tmp.path().join("checkouts").join("co-dev");
        let now = chrono::DateTime::parse_from_rfc3339("2026-04-27T00:00:00Z")
            .unwrap()
            .timestamp_nanos_opt()
            .unwrap();

        let stats =
            sync_universe_git(store.as_ref(), "co-dev", &remote, "main", &dest, None, now).unwrap();
        assert_eq!(stats.commits_ingested, 1);
        assert!(stats.head_sha.is_some());

        // Commit entry + profile + analytics all present.
        let profile = store.get("profiles/yuri.md").unwrap().unwrap();
        assert_eq!(profile.frontmatter["analytics"]["commits_total"], 1);
        let commits = store.list("commit", &serde_json::json!({}), None).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].frontmatter["parent_task"], "CO-65");
    }
}
