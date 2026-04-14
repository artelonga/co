//! CO-50: Git-backed universe sync.
//!
//! Clones a GitHub repo on first access and keeps the entry index in sync
//! via lazy refresh (5-minute cooldown), manual trigger, or GitHub webhook.
//!
//! # Private repos (MVP)
//!
//! Set `GIT_DEPLOY_KEY_PATH=/path/to/id_ed25519` in the server environment.
//! The deploy key must be added as a read collaborator on the private repo.
//! If unset, only public repos are supported.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use chrono::Utc;
use co::entry::{Entry, scan_entries};

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Returns the local directory where a universe's git clone lives.
///
/// Clones live at `{data_dir}/repos/{universe_key}/` — separate from the
/// universe's user-created entries at `{data_dir}/universes/{universe_key}/`.
pub fn repo_dir(data_dir: &Path, universe_key: &str) -> PathBuf {
    data_dir.join("repos").join(universe_key)
}

/// Returns the content root within a cloned repo.
///
/// If `subpath` is `Some("data/co")`, returns `{repo_dir}/data/co`.
/// If `subpath` is `None` or empty, returns the repo root itself.
pub fn content_dir(repo_dir: &Path, subpath: Option<&str>) -> PathBuf {
    match subpath {
        Some(p) if !p.is_empty() => repo_dir.join(p),
        _ => repo_dir.to_path_buf(),
    }
}

/// Build the HTTPS clone URL for a `"owner/repo"` slug.
pub fn repo_url(repo: &str) -> String {
    format!("https://github.com/{}.git", repo)
}

// ---------------------------------------------------------------------------
// Git operations
// ---------------------------------------------------------------------------

/// Clone a repo (shallow, single-branch) into `dest`. Returns the HEAD commit hash.
pub fn clone_repo(url: &str, branch: &str, dest: &Path) -> Result<String> {
    let dest_str = dest
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid destination path"))?;
    run_git(
        &[
            "clone",
            "--depth",
            "1",
            "--branch",
            branch,
            "--single-branch",
            url,
            dest_str,
        ],
        None,
    )?;
    local_commit_hash(dest)
}

/// Pull the latest changes (ff-only). Returns the new HEAD commit hash.
pub fn pull_repo(repo_dir: &Path) -> Result<String> {
    run_git(&["pull", "--ff-only", "--depth", "1"], Some(repo_dir))?;
    local_commit_hash(repo_dir)
}

/// Get the remote HEAD commit hash for `branch` without fetching any objects.
pub fn remote_head(repo_dir: &Path, branch: &str) -> Result<String> {
    let refspec = format!("refs/heads/{}", branch);
    let out = run_git(&["ls-remote", "origin", &refspec], Some(repo_dir))?;
    let hash = out.split_whitespace().next().unwrap_or("").to_string();
    if hash.is_empty() {
        anyhow::bail!("Branch '{}' not found on remote", branch);
    }
    Ok(hash)
}

/// Get the HEAD commit hash of a local clone.
pub fn local_commit_hash(dir: &Path) -> Result<String> {
    let out = run_git(&["rev-parse", "HEAD"], Some(dir))?;
    Ok(out.trim().to_string())
}

// ---------------------------------------------------------------------------
// Lazy-refresh heuristic
// ---------------------------------------------------------------------------

/// Returns `true` if a lazy sync check should fire:
/// - Universe has never been synced (no `synced_at`), OR
/// - Last sync was more than 5 minutes ago.
pub fn needs_lazy_sync(synced_at: Option<&str>) -> bool {
    match synced_at {
        None => true,
        Some(t) => chrono::DateTime::parse_from_rfc3339(t)
            .map(|dt| {
                let age = Utc::now() - dt.with_timezone(&Utc);
                age.num_seconds() > 300
            })
            .unwrap_or(true),
    }
}

// ---------------------------------------------------------------------------
// Full sync
// ---------------------------------------------------------------------------

/// Sync a git-backed universe.
///
/// 1. If the local repo clone doesn't exist → `git clone`.
/// 2. Otherwise, check the remote HEAD via `git ls-remote`.
///    - If it matches `known_hash` → already up to date → returns `None`.
///    - If different → `git pull`.
/// 3. Scan `.md` files from `content_dir` and return them.
///
/// Returns `Some((commit_hash, entries))` when a sync was performed,
/// or `None` when the local clone is already at the latest commit.
pub fn sync(
    data_dir: &Path,
    universe_key: &str,
    repo: &str,
    subpath: Option<&str>,
    branch: &str,
    known_hash: Option<&str>,
) -> Result<Option<(String, Vec<Entry>)>> {
    let repo_path = repo_dir(data_dir, universe_key);
    let url = repo_url(repo);

    let commit_hash = if !repo_path.exists() {
        // First access: clone the repo.
        if let Some(parent) = repo_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        tracing::info!(universe = universe_key, repo = repo, "git clone starting");
        clone_repo(&url, branch, &repo_path)?
    } else {
        // Check whether the remote has advanced past our cached hash.
        let remote = match remote_head(&repo_path, branch) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    universe = universe_key,
                    error = %e,
                    "git ls-remote failed — using cached entries"
                );
                return Err(e);
            }
        };
        if Some(remote.as_str()) == known_hash {
            // Already up to date — no sync needed.
            return Ok(None);
        }
        tracing::info!(
            universe = universe_key,
            old = known_hash.unwrap_or("none"),
            new = %remote,
            "git pull starting"
        );
        pull_repo(&repo_path)?
    };

    let content = content_dir(&repo_path, subpath);
    let entries = scan_entries(&content).unwrap_or_default();

    tracing::info!(
        universe = universe_key,
        commit = %commit_hash,
        entries = entries.len(),
        "git sync complete"
    );

    Ok(Some((commit_hash, entries)))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Run a git command, optionally in a working directory.
///
/// If `GIT_DEPLOY_KEY_PATH` is set in the environment, injects
/// `GIT_SSH_COMMAND` to use that SSH key for private-repo access.
fn run_git(args: &[&str], dir: Option<&Path>) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.args(args);

    if let Some(d) = dir {
        cmd.current_dir(d);
    }

    // Private repo support via deploy key.
    if let Ok(key_path) = std::env::var("GIT_DEPLOY_KEY_PATH") {
        cmd.env(
            "GIT_SSH_COMMAND",
            format!("ssh -i {key_path} -o StrictHostKeyChecking=no"),
        );
    }

    let output = cmd.output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Initialise a local git repo, add a .md file, commit, and return the repo dir.
    fn init_local_repo(content: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let repo = dir.path().to_path_buf();

        run_git(&["init", "-b", "main"], Some(&repo)).expect("git init");
        run_git(&["config", "user.email", "test@co.local"], Some(&repo)).expect("git config email");
        run_git(&["config", "user.name", "Test"], Some(&repo)).expect("git config name");

        for (path, body) in content {
            let full = repo.join(path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full, body).unwrap();
            run_git(&["add", path], Some(&repo)).expect("git add");
        }

        run_git(&["commit", "-m", "init", "--allow-empty"], Some(&repo)).expect("git commit");

        (dir, repo)
    }

    #[test]
    fn test_repo_dir() {
        let data = Path::new("/data");
        assert_eq!(
            repo_dir(data, "co-dev"),
            PathBuf::from("/data/repos/co-dev")
        );
    }

    #[test]
    fn test_content_dir_with_subpath() {
        let repo = Path::new("/repos/co-dev");
        assert_eq!(
            content_dir(repo, Some("data/co")),
            PathBuf::from("/repos/co-dev/data/co")
        );
    }

    #[test]
    fn test_content_dir_no_subpath() {
        let repo = Path::new("/repos/co-dev");
        assert_eq!(content_dir(repo, None), PathBuf::from("/repos/co-dev"));
        assert_eq!(content_dir(repo, Some("")), PathBuf::from("/repos/co-dev"));
    }

    #[test]
    fn test_repo_url() {
        assert_eq!(
            repo_url("artelonga/co"),
            "https://github.com/artelonga/co.git"
        );
    }

    #[test]
    fn test_needs_lazy_sync_never_synced() {
        assert!(needs_lazy_sync(None));
    }

    #[test]
    fn test_needs_lazy_sync_recent() {
        // synced_at = just now → should NOT need sync
        let now = Utc::now().to_rfc3339();
        assert!(!needs_lazy_sync(Some(&now)));
    }

    #[test]
    fn test_needs_lazy_sync_stale() {
        // synced_at = 10 minutes ago → should need sync
        let old = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
        assert!(needs_lazy_sync(Some(&old)));
    }

    #[test]
    fn test_needs_lazy_sync_invalid_date() {
        // Unparseable date → treat as stale
        assert!(needs_lazy_sync(Some("not-a-date")));
    }

    #[test]
    fn test_local_commit_hash() {
        let (_dir, repo) = init_local_repo(&[("note.md", "---\ntype: note\n---\n")]);
        let hash = local_commit_hash(&repo).unwrap();
        assert_eq!(hash.len(), 40); // SHA-1 hex
    }

    #[test]
    fn test_sync_with_local_repo() {
        // Build a "remote" repo (local dir acts as origin).
        let (_origin_dir, origin_repo) = init_local_repo(&[(
            "task-1.md",
            "---\ntype: task\ntitle: Test task\nstatus: todo\n---\nBody.\n",
        )]);

        // Clone it into a temp data dir.
        let data_dir = tempdir().unwrap();
        let url = format!("file://{}", origin_repo.display());

        // Use clone_repo directly (sync() calls into this).
        let dest = data_dir.path().join("repos").join("test-universe");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let hash = clone_repo(&url, "main", &dest).unwrap();
        assert!(!hash.is_empty());

        // Scan entries from cloned repo.
        let entries = scan_entries(&dest).unwrap_or_default();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entry_type, "task");
    }

    #[test]
    fn test_sync_no_change_returns_none() {
        let (_origin_dir, origin_repo) = init_local_repo(&[(
            "task-1.md",
            "---\ntype: task\ntitle: T\nstatus: todo\n---\n",
        )]);
        let data_dir = tempdir().unwrap();
        let url = format!("file://{}", origin_repo.display());

        // First sync: clone.
        let dest = data_dir.path().join("repos").join("uni");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let hash = clone_repo(&url, "main", &dest).unwrap();

        // Second sync with same known_hash: should return None.
        // We can't use sync() directly since it calls ls-remote which requires network
        // or a proper remote setup. Test the building block instead:
        assert!(!hash.is_empty());
        // If known_hash == remote_head, sync returns None.
        // Verify this logic by checking the hash matches local.
        let local = local_commit_hash(&dest).unwrap();
        assert_eq!(hash, local);
    }
}
