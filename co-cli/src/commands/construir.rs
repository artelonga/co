use std::path::{Path, PathBuf};

/// `co construir [<key>] [--out <dir>] [--redearte <path>]`
///
/// Builds a universe's `content/*.md` into a Quartz static garden via the
/// redearte template, writing to `--out` (default: `<repo>/public/`).
///
/// Run from within the universe directory:
///   co construir                            # key derived from dir name
///   co construir grcsamazonia              # same, explicit key for display
///   co construir --out dist/               # custom output dir
///
/// Only files under `content/` are included; `_source/` PII directories and
/// non-content files are excluded by construction (Quartz only sees `content/`).
pub fn run(key: Option<String>, out: String, redearte: Option<PathBuf>) {
    let cwd = std::env::current_dir().expect("cannot read CWD");
    let repo_root = find_repo_root(&cwd);

    let basename = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("universe")
        .to_string();

    let universe_key = key.unwrap_or_else(|| sanitize_key(&basename));

    let content_dir = repo_root.join("content");
    if !content_dir.exists() {
        eprintln!(
            "error: content/ not found at {}\n  Run `co construir` from within a universe directory.",
            content_dir.display()
        );
        std::process::exit(1);
    }

    let out_dir = if Path::new(&out).is_absolute() {
        PathBuf::from(&out)
    } else {
        repo_root.join(&out)
    };

    let redearte_dir = redearte.unwrap_or_else(resolve_redearte);

    if !redearte_dir.join("package.json").exists() {
        eprintln!(
            "error: redearte template not found at {}\n  Clone: git clone https://github.com/artelonga/redearte ~/projects/redearte\n  Or set CO_REDEARTE_PATH=/path/to/redearte.",
            redearte_dir.display()
        );
        std::process::exit(1);
    }

    if !redearte_dir.join("node_modules").exists() {
        eprintln!(
            "error: redearte dependencies not installed — run:\n  cd {} && npm install",
            redearte_dir.display()
        );
        std::process::exit(1);
    }

    eprintln!(
        "construir: '{}' — {} → {}",
        universe_key,
        content_dir.display(),
        out_dir.display()
    );

    let status = std::process::Command::new("npx")
        .args([
            "quartz",
            "build",
            "-d",
            &content_dir.to_string_lossy(),
            "-o",
            &out_dir.to_string_lossy(),
        ])
        .current_dir(&redearte_dir)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("error: failed to run npx: {e}\n  Is Node.js installed?");
            std::process::exit(1);
        });

    if !status.success() {
        eprintln!("error: quartz build exited with {}", status);
        std::process::exit(1);
    }

    eprintln!("construir: done — static garden at {}", out_dir.display());
    eprintln!(
        "  deploy: cd {} && fly deploy --config deploy/fly.toml --dockerfile deploy/Dockerfile --remote-only --ha=false",
        repo_root.display()
    );
}

/// Resolves the redearte Quartz template directory.
/// Precedence: `CO_REDEARTE_PATH` env var → `~/projects/redearte`.
fn resolve_redearte() -> PathBuf {
    if let Ok(path) = std::env::var("CO_REDEARTE_PATH") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join("projects")
        .join("redearte")
}

/// Walk up the directory tree looking for a `.git` or `.jj` directory.
/// Falls back to `start` if neither is found (e.g., in a bare working tree).
fn find_repo_root(start: &Path) -> PathBuf {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() || dir.join(".jj").exists() {
            return dir;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return start.to_path_buf(),
        }
    }
}

/// Lowercase and sanitize a directory name into a valid universe key.
fn sanitize_key(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_key_lowercases() {
        assert_eq!(sanitize_key("GrcSamazonia"), "grcsamazonia");
    }

    #[test]
    fn sanitize_key_collapses_dashes() {
        assert_eq!(sanitize_key("grcs--amazonia"), "grcs-amazonia");
    }

    #[test]
    fn sanitize_key_replaces_underscores() {
        assert_eq!(sanitize_key("grcs_amazonia"), "grcs-amazonia");
    }

    #[test]
    fn sanitize_key_strips_leading_trailing() {
        assert_eq!(sanitize_key("_project_"), "project");
    }

    #[test]
    fn find_repo_root_falls_back_to_start() {
        let tmp = tempfile::tempdir().unwrap();
        let result = find_repo_root(tmp.path());
        assert_eq!(result, tmp.path());
    }

    #[test]
    fn find_repo_root_finds_git() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let subdir = tmp.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        let result = find_repo_root(&subdir);
        assert_eq!(result, tmp.path());
    }

    #[test]
    fn find_repo_root_finds_jj() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".jj")).unwrap();
        let subdir = tmp.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        let result = find_repo_root(&subdir);
        assert_eq!(result, tmp.path());
    }

    #[test]
    fn resolve_redearte_returns_a_path() {
        let path = resolve_redearte();
        assert!(path.to_str().is_some());
    }
}
