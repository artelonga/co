use co_web::storage::Storage;

/// `co launch` — provision a universe from the current directory, Fly-style.
///
/// Walks up from CWD to find the git repo root (falls back to CWD if there is
/// no `.git`), derives a universe key from the directory name, and seeds
/// `docs/`, `content/`, and `work/` into localhost CO storage.
pub fn run(
    key: Option<String>,
    name: Option<String>,
    public: bool,
    now: bool,
    port: u16,
    data_dir: String,
) {
    let cwd = std::env::current_dir().expect("cannot read CWD");
    let repo_root = find_repo_root(&cwd);

    let basename = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("universe")
        .to_string();

    let universe_key = key.unwrap_or_else(|| sanitize_key(&basename));
    let display_name = name.unwrap_or_else(|| basename.clone());

    let mut storage = Storage::new(&data_dir);

    storage.ensure_local_universe(&universe_key, &display_name, public);

    // CO-330: wire the universe to its source directory so seed_orchestrator
    // can re-ingest on the next `co serve` restart without a hardcoded list.
    let repo_path_str = repo_root.to_string_lossy().to_string();
    if let Err(e) =
        storage.update_universe_source(&universe_key, Some(&repo_path_str), None, None, None, None)
    {
        eprintln!("warning: could not set local_repo_path: {e}");
    }

    storage.seed_universe_from_local_repo(&universe_key, &repo_root, &["docs", "content"], 0);

    let work_root = repo_root.join("work");
    storage.seed_universe_work_tasks_from_local(&universe_key, &work_root, 0);

    let (pages, tasks, projects) = storage.count_universe_entries(&universe_key);

    eprintln!(
        "Universe '{}' provisioned: {} pages, {} tasks across {} projects",
        universe_key, pages, tasks, projects
    );

    if now {
        let url = format!("http://localhost:{}/{}", port, universe_key);
        eprintln!("Serving on {}", url);
        launch_browser(&url);
        super::serve::run(port, data_dir, false, false);
    } else {
        eprintln!(
            "Run `co serve --open` to browse http://localhost:{}/{}",
            port, universe_key
        );
    }
}

/// Walk up the directory tree looking for a `.git` directory.
/// Falls back to `start` if no git root is found.
fn find_repo_root(start: &std::path::Path) -> std::path::PathBuf {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return dir;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return start.to_path_buf(),
        }
    }
}

/// Lowercase and sanitize a directory name into a valid universe key.
/// Non-alphanumeric characters (except `-`) are replaced with `-`.
/// Consecutive dashes are collapsed; leading/trailing dashes are stripped.
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

fn launch_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();

    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();

    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn();

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let _ = url;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_key_lowercases() {
        assert_eq!(sanitize_key("MyProject"), "myproject");
    }

    #[test]
    fn sanitize_key_preserves_dashes() {
        assert_eq!(sanitize_key("rfq-gateway"), "rfq-gateway");
    }

    #[test]
    fn sanitize_key_replaces_underscores() {
        assert_eq!(sanitize_key("my_project"), "my-project");
    }

    #[test]
    fn sanitize_key_collapses_dashes() {
        assert_eq!(sanitize_key("my--project"), "my-project");
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
        let subdir = tmp.path().join("src");
        std::fs::create_dir(&subdir).unwrap();
        let result = find_repo_root(&subdir);
        assert_eq!(result, tmp.path());
    }
}
