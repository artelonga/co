//! VCS helpers for remote sister-repo sync (CO-337).
//!
//! Provides `clone` and `pull` operations backed by the system `git` binary.
//! Used by `run_remote_sister_repo_seeds` to keep sister-repo clones fresh
//! on the production machine where local paths don't exist.

use std::path::Path;
use std::process::Command;

/// Shallow-clone `remote_url` into `dest` (must not yet exist).
///
/// Uses `--depth=1` and `--no-recurse-submodules` to keep the clone small.
/// Authentication is applied via env vars — see [`apply_auth_env`].
pub fn clone(remote_url: &str, dest: &Path) -> anyhow::Result<()> {
    let mut cmd = Command::new("git");
    cmd.args([
        "clone",
        "--depth=1",
        "--no-recurse-submodules",
        "--",
        remote_url,
    ]);
    cmd.arg(dest);
    apply_auth_env(&mut cmd);
    let status = cmd.status()?;
    anyhow::ensure!(
        status.success(),
        "git clone failed for {remote_url} (exit {})",
        status.code().unwrap_or(-1)
    );
    Ok(())
}

/// Fetch `ref_name` from origin and reset the working tree to it.
///
/// Equivalent to `git fetch --depth=1 origin <ref_name> &&
/// git reset --hard origin/<ref_name>`.
pub fn pull(dest: &Path, ref_name: &str) -> anyhow::Result<()> {
    let mut fetch_cmd = Command::new("git");
    fetch_cmd
        .current_dir(dest)
        .args(["fetch", "--depth=1", "origin", ref_name]);
    apply_auth_env(&mut fetch_cmd);
    let status = fetch_cmd.status()?;
    anyhow::ensure!(
        status.success(),
        "git fetch failed in {} (exit {})",
        dest.display(),
        status.code().unwrap_or(-1)
    );

    let refspec = format!("origin/{ref_name}");
    let status = Command::new("git")
        .current_dir(dest)
        .args(["reset", "--hard", &refspec])
        .status()?;
    anyhow::ensure!(
        status.success(),
        "git reset --hard {refspec} failed in {} (exit {})",
        dest.display(),
        status.code().unwrap_or(-1)
    );
    Ok(())
}

/// Clone `remote_url` into `dest` if it does not exist; otherwise pull.
///
/// Creates `dest`'s parent directory if needed.
pub fn sync_repo(remote_url: &str, ref_name: &str, dest: &Path) -> anyhow::Result<()> {
    if dest.exists() {
        pull(dest, ref_name)
    } else {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        clone(remote_url, dest)
    }
}

/// Whether any git credential is configured in the environment.
///
/// True when either `CO_GIT_TOKEN` (HTTPS) or `CO_GIT_SSH_KEY_PATH` (SSH) is
/// set and non-empty. CO-408.
pub fn has_git_credentials() -> bool {
    let nonempty = |k: &str| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false);
    nonempty("CO_GIT_TOKEN") || nonempty("CO_GIT_SSH_KEY_PATH")
}

/// Whether `remote_url` points at a network host (vs a local path / `file://`).
///
/// Network remotes (`https://`, `http://`, `ssh://`, `git://`, or `git@host:`
/// scp-style) require authentication for private repos. Local remotes (`file://`,
/// bare filesystem paths used in tests/dev) never do. CO-408.
fn is_network_remote(remote_url: &str) -> bool {
    remote_url.starts_with("https://")
        || remote_url.starts_with("http://")
        || remote_url.starts_with("ssh://")
        || remote_url.starts_with("git://")
        // scp-style: `git@github.com:org/repo.git` (a `:` before any `/`).
        || (remote_url.contains('@')
            && remote_url
                .split_once(':')
                .map(|(host, _)| !host.contains('/'))
                .unwrap_or(false))
}

/// Whether syncing `remote_url` would prompt git for credentials we don't have.
///
/// With no credential configured, a network remote makes git emit
/// `fatal: could not read Username for 'https://...'` on the prod machine and the
/// sync is doomed anyway. CO-408.
pub fn remote_needs_credentials(remote_url: &str) -> bool {
    is_network_remote(remote_url) && !has_git_credentials()
}

/// Inject authentication credentials into a git `Command` via env vars.
///
/// Supported env vars:
/// - `CO_GIT_SSH_KEY_PATH` — path to an SSH private key (e.g. a Fly secret file mount)
/// - `CO_GIT_TOKEN` — GitHub personal access token for HTTPS clones
fn apply_auth_env(cmd: &mut Command) {
    if let Ok(key_path) = std::env::var("CO_GIT_SSH_KEY_PATH") {
        cmd.env(
            "GIT_SSH_COMMAND",
            format!("ssh -i {key_path} -o StrictHostKeyChecking=no -o BatchMode=yes"),
        );
    }
    if let Ok(token) = std::env::var("CO_GIT_TOKEN") {
        // Suppress interactive prompts and inject token via HTTP extra header.
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("GIT_ASKPASS", "true");
        cmd.env("GIT_CONFIG_COUNT", "1");
        cmd.env("GIT_CONFIG_KEY_0", "http.extraheader");
        cmd.env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: token {token}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn git_available() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    /// Set up a local bare "origin" with one commit and return (origin_path, work_path).
    ///
    /// CO-337 CI fix: don't clone the empty bare repo (which fails on the GH
    /// Actions runner where git's defaults differ). Instead, init `work` as a
    /// fresh repo with explicit `main` branch + add remote + push. The bare
    /// is also init'd with `--initial-branch=main` so HEAD resolves correctly
    /// for subsequent clones in the tests under test.
    fn init_local_repo(tmp: &tempfile::TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
        let origin = tmp.path().join("origin.git");
        let work = tmp.path().join("work");

        // Bare origin with main as initial branch (works on any git version
        // that supports --initial-branch; available since git 2.28).
        Command::new("git")
            .args([
                "init",
                "--bare",
                "--initial-branch=main",
                origin.to_str().unwrap(),
            ])
            .output()
            .expect("git init --bare");

        // Init work fresh (no clone of empty bare — that misbehaves on CI).
        std::fs::create_dir_all(&work).expect("create work dir");
        Command::new("git")
            .current_dir(&work)
            .args(["init", "--initial-branch=main"])
            .output()
            .expect("git init work");
        for (k, v) in [("user.email", "test@co"), ("user.name", "CO Test")] {
            Command::new("git")
                .current_dir(&work)
                .args(["config", k, v])
                .output()
                .expect("git config");
        }
        Command::new("git")
            .current_dir(&work)
            .args(["remote", "add", "origin", origin.to_str().unwrap()])
            .output()
            .expect("git remote add");

        std::fs::write(work.join("README.md"), "hello").expect("write README");
        Command::new("git")
            .current_dir(&work)
            .args(["add", "."])
            .output()
            .expect("git add");
        Command::new("git")
            .current_dir(&work)
            .args(["commit", "-m", "init"])
            .output()
            .expect("git commit");
        Command::new("git")
            .current_dir(&work)
            .args(["push", "-u", "origin", "main"])
            .output()
            .expect("git push");
        (origin, work)
    }

    #[test]
    fn network_remotes_are_detected() {
        assert!(is_network_remote("https://github.com/artelonga/mbya"));
        assert!(is_network_remote("http://example.com/repo.git"));
        assert!(is_network_remote("ssh://git@github.com/org/repo.git"));
        assert!(is_network_remote("git://example.com/repo.git"));
        assert!(is_network_remote("git@github.com:artelonga/mbya.git"));
    }

    #[test]
    fn local_remotes_are_not_network() {
        assert!(!is_network_remote("file:///data/origin.git"));
        assert!(!is_network_remote("/tmp/xyz/origin.git"));
        assert!(!is_network_remote("./relative/origin.git"));
        // A bare path with no scheme and no scp-style colon.
        assert!(!is_network_remote("origin.git"));
    }

    #[test]
    fn clone_creates_dest_with_files() {
        if !git_available() {
            return;
        }
        let tmp = tempdir().unwrap();
        let (origin, _) = init_local_repo(&tmp);
        let dest = tmp.path().join("checkout");
        clone(origin.to_str().unwrap(), &dest).unwrap();
        assert!(dest.join("README.md").exists());
    }

    #[test]
    fn pull_fetches_new_commit() {
        if !git_available() {
            return;
        }
        let tmp = tempdir().unwrap();
        let (origin, work) = init_local_repo(&tmp);
        let dest = tmp.path().join("checkout");
        clone(origin.to_str().unwrap(), &dest).unwrap();

        // Push a second commit to origin.
        std::fs::write(work.join("file2.txt"), "world").expect("write file2");
        Command::new("git")
            .current_dir(&work)
            .args(["add", "."])
            .output()
            .expect("git add");
        Command::new("git")
            .current_dir(&work)
            .args(["commit", "-m", "add file2"])
            .output()
            .expect("git commit");
        Command::new("git")
            .current_dir(&work)
            .args(["push", "origin", "HEAD:main"])
            .output()
            .expect("git push");

        pull(&dest, "main").unwrap();
        assert!(dest.join("file2.txt").exists());
    }

    #[test]
    fn sync_repo_clones_when_dest_missing() {
        if !git_available() {
            return;
        }
        let tmp = tempdir().unwrap();
        let (origin, _) = init_local_repo(&tmp);
        let dest = tmp.path().join("nested").join("checkout");
        assert!(!dest.exists());
        sync_repo(origin.to_str().unwrap(), "main", &dest).unwrap();
        assert!(dest.join("README.md").exists());
    }

    #[test]
    fn sync_repo_pulls_when_dest_exists() {
        if !git_available() {
            return;
        }
        let tmp = tempdir().unwrap();
        let (origin, work) = init_local_repo(&tmp);
        let dest = tmp.path().join("checkout");
        // First clone via sync_repo.
        sync_repo(origin.to_str().unwrap(), "main", &dest).unwrap();
        assert!(!dest.join("file2.txt").exists());

        // Push a second commit.
        std::fs::write(work.join("file2.txt"), "world").expect("write file2");
        Command::new("git")
            .current_dir(&work)
            .args(["add", "."])
            .output()
            .expect("git add");
        Command::new("git")
            .current_dir(&work)
            .args(["commit", "-m", "add file2"])
            .output()
            .expect("git commit");
        Command::new("git")
            .current_dir(&work)
            .args(["push", "origin", "HEAD:main"])
            .output()
            .expect("git push");

        // Second sync_repo should pull not clone.
        sync_repo(origin.to_str().unwrap(), "main", &dest).unwrap();
        assert!(dest.join("file2.txt").exists());
    }
}
