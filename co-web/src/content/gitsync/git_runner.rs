//! CO-89: thin wrappers around the system `git` binary for full-history log
//! ingestion. Kept separate from the pure parser ([`super::git_log`]) so the
//! side-effecting bits are isolated; tested against a local `file://` repo (no
//! network).
//!
//! Unlike [`crate::vcs`] (which shallow-clones sister-repo *content*), git-sync
//! needs the **whole commit history**, so these helpers never pass `--depth`.

use std::path::Path;
use std::process::Command;

use super::git_log::GIT_LOG_FORMAT;

/// Clone `remote` into `dest` (full history) if absent; otherwise fetch + reset
/// to `origin/<branch>`. Creates `dest`'s parent as needed.
pub fn ensure_repo(remote: &str, branch: &str, dest: &Path) -> anyhow::Result<()> {
    if dest.exists() {
        run_git(dest, &["fetch", "origin", branch])?;
        run_git(dest, &["reset", "--hard", &format!("origin/{branch}")])?;
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Full clone (no --depth) so `git log` can walk the entire history.
        run_git_cwd(
            None,
            &["clone", "--no-recurse-submodules", "--", remote],
            dest,
        )?;
    }
    Ok(())
}

/// Run `git log <branch>` with the CO-89 machine format + `--shortstat` and
/// return raw stdout for [`super::git_log::parse_git_log`].
pub fn run_git_log(repo_dir: &Path, branch: &str) -> anyhow::Result<String> {
    let format_arg = format!("--pretty=format:{GIT_LOG_FORMAT}");
    let out = Command::new("git")
        .current_dir(repo_dir)
        .args(["log", branch, "--shortstat", &format_arg])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    anyhow::ensure!(
        out.status.success(),
        "git log failed in {} (exit {}): {}",
        repo_dir.display(),
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run_git(dir: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()?;
    anyhow::ensure!(
        status.success(),
        "git {} failed in {} (exit {})",
        args.join(" "),
        dir.display(),
        status.code().unwrap_or(-1)
    );
    Ok(())
}

/// `git clone` runs without a working directory; the destination is the last
/// positional arg.
fn run_git_cwd(cwd: Option<&Path>, args: &[&str], dest: &Path) -> anyhow::Result<()> {
    let mut cmd = Command::new("git");
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    cmd.args(args).arg(dest).env("GIT_TERMINAL_PROMPT", "0");
    let status = cmd.status()?;
    anyhow::ensure!(
        status.success(),
        "git {} {} failed (exit {})",
        args.join(" "),
        dest.display(),
        status.code().unwrap_or(-1)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::gitsync::git_log::parse_git_log;
    use std::process::Command;
    use tempfile::tempdir;

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    /// Create a local repo with two CO-style commits on `main`, deterministic
    /// dates via GIT_*_DATE so the test never depends on the wall clock.
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
        std::fs::write(dir.join("a.txt"), "hello\nworld\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-m", "feat(co-web): CO-65 first"]);
        std::fs::write(dir.join("b.txt"), "more\n").unwrap();
        g(&["add", "."]);
        g(&["commit", "-m", "fix: CO-66 second"]);
    }

    #[test]
    fn run_git_log_against_real_repo_parses() {
        if !git_available() {
            return;
        }
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);

        let raw = run_git_log(&repo, "main").unwrap();
        let recs = parse_git_log(&raw);
        assert_eq!(recs.len(), 2, "two commits ingested");
        // Newest-first: CO-66 then CO-65.
        assert_eq!(recs[0].parent_task.as_deref(), Some("CO-66"));
        assert_eq!(recs[1].parent_task.as_deref(), Some("CO-65"));
        assert_eq!(recs[0].author_handle, "yuri");
        assert!(recs[1].files_changed >= 1);
    }

    #[test]
    fn ensure_repo_clones_then_fetches() {
        if !git_available() {
            return;
        }
        let tmp = tempdir().unwrap();
        let origin = tmp.path().join("origin");
        init_repo(&origin);
        let remote = format!("file://{}", origin.display());

        let dest = tmp.path().join("clone");
        ensure_repo(&remote, "main", &dest).unwrap();
        let recs = parse_git_log(&run_git_log(&dest, "main").unwrap());
        assert_eq!(recs.len(), 2);

        // Second ensure_repo on an existing clone fetches (no error, no re-clone).
        ensure_repo(&remote, "main", &dest).unwrap();
        assert!(dest.join("a.txt").exists());
    }
}
