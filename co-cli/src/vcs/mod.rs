//! VCS backend abstraction — git vs jujutsu (CO-331).
//!
//! Detects the backend by checking for `.jj/` (jujutsu) or falling back to git.
//! All operations use the CLI of whichever backend is active at the given path.

use std::path::Path;
use std::process::Command;

/// Which VCS backend manages a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Git,
    Jujutsu,
}

/// Detect the VCS backend for an existing checkout directory.
/// Returns `Jujutsu` when `.jj/` exists, `Git` otherwise.
pub fn detect_backend(path: &Path) -> Backend {
    if path.join(".jj").exists() {
        Backend::Jujutsu
    } else {
        Backend::Git
    }
}

/// Clone `remote_url` into `dest` using git.
/// Initial clones always use `git clone` — jujutsu repos may add a `.jj/`
/// layer on top afterwards, and subsequent operations will dispatch via `jj`.
pub fn clone_repo(remote_url: &str, dest: &Path) -> anyhow::Result<()> {
    let status = Command::new("git")
        .args(["clone", "--", remote_url])
        .arg(dest)
        .status()?;
    anyhow::ensure!(
        status.success(),
        "git clone failed (exit {})",
        status.code().unwrap_or(-1)
    );
    Ok(())
}

/// Fetch the latest refs from the remote, dispatching to git or jj.
pub fn fetch(dest: &Path) -> anyhow::Result<()> {
    let status = match detect_backend(dest) {
        Backend::Jujutsu => Command::new("jj")
            .current_dir(dest)
            .args(["git", "fetch"])
            .status()?,
        Backend::Git => Command::new("git")
            .current_dir(dest)
            .args(["fetch", "--all", "--prune"])
            .status()?,
    };
    anyhow::ensure!(
        status.success(),
        "fetch failed (exit {})",
        status.code().unwrap_or(-1)
    );
    Ok(())
}

/// Check out `ref_name` (tag, branch, SHA) in `dest`.
pub fn checkout(dest: &Path, ref_name: &str) -> anyhow::Result<()> {
    let status = match detect_backend(dest) {
        Backend::Jujutsu => Command::new("jj")
            .current_dir(dest)
            .args(["new", ref_name])
            .status()?,
        Backend::Git => Command::new("git")
            .current_dir(dest)
            .args(["checkout", ref_name])
            .status()?,
    };
    anyhow::ensure!(
        status.success(),
        "checkout '{}' failed (exit {})",
        ref_name,
        status.code().unwrap_or(-1)
    );
    Ok(())
}

/// Return the current HEAD commit SHA for `dest`.
pub fn head_sha(dest: &Path) -> anyhow::Result<String> {
    let output = match detect_backend(dest) {
        Backend::Jujutsu => Command::new("jj")
            .current_dir(dest)
            .args(["log", "-r", "@", "-T", "commit_id", "--no-graph"])
            .output()?,
        Backend::Git => Command::new("git")
            .current_dir(dest)
            .args(["rev-parse", "HEAD"])
            .output()?,
    };
    anyhow::ensure!(
        output.status.success(),
        "failed to read HEAD SHA (exit {})",
        output.status.code().unwrap_or(-1)
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detects_git_when_no_jj() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert_eq!(detect_backend(dir.path()), Backend::Git);
    }

    #[test]
    fn detects_jujutsu_when_jj_exists() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".jj")).unwrap();
        assert_eq!(detect_backend(dir.path()), Backend::Jujutsu);
    }

    #[test]
    fn defaults_to_git_when_empty_dir() {
        let dir = tempdir().unwrap();
        assert_eq!(detect_backend(dir.path()), Backend::Git);
    }

    #[test]
    fn jj_takes_precedence_over_git() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::create_dir(dir.path().join(".jj")).unwrap();
        assert_eq!(detect_backend(dir.path()), Backend::Jujutsu);
    }
}
