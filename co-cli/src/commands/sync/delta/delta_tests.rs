//! CO-91 delta-push engine tests.
//!
//! The planning/changelog/config logic runs in-process against fake
//! [`DeltaSource`]/[`Uploader`] impls (no `jj`, keychain or network). One
//! `jj`-backed test (skipped when `jj` is absent) verifies that the planned
//! upload set matches `jj diff --name-only`.

use std::cell::RefCell;
use std::process::Command;

use super::*;
use tempfile::TempDir;

// `co_home()` reads a process-global env var; serialize the tests that set it.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ─── Glob matcher ───────────────────────────────────────────────────────────

#[test]
fn glob_double_star_matches_any_depth() {
    assert!(glob_match("**/*.md", "a.md"));
    assert!(glob_match("**/*.md", "dir/a.md"));
    assert!(glob_match("**/*.md", "a/b/c/d.md"));
    assert!(!glob_match("**/*.md", "a.txt"));
    assert!(!glob_match("**/*.md", "dir/a.txt"));
}

#[test]
fn glob_prefix_dir_matches_subtree() {
    assert!(glob_match("node_modules/**", "node_modules/x/y.md"));
    assert!(glob_match("node_modules/**", "node_modules/pkg.json"));
    assert!(!glob_match("node_modules/**", "src/node_modules.md"));
}

#[test]
fn glob_single_star_does_not_cross_slash() {
    assert!(glob_match("*.md", "a.md"));
    assert!(!glob_match("*.md", "dir/a.md"));
}

#[test]
fn file_filter_default_excludes_build_dirs() {
    let f = FileFilter::new(None, None);
    assert!(f.matches("content/note.md"));
    assert!(f.matches("a/b/deep.md"));
    assert!(!f.matches("note.txt"));
    assert!(!f.matches("node_modules/pkg/readme.md"));
    assert!(!f.matches("build/out.md"));
    assert!(!f.matches("target/doc/x.md"));
    assert!(!f.matches(".jj/repo/op.md"));
}

#[test]
fn file_filter_honors_custom_include_exclude() {
    let f = FileFilter::new(
        Some(vec!["docs/**/*.md".to_string()]),
        Some(vec!["docs/private/**".to_string()]),
    );
    assert!(f.matches("docs/guide.md"));
    assert!(f.matches("docs/sub/guide.md"));
    assert!(!f.matches("README.md"));
    assert!(!f.matches("docs/private/secret.md"));
}

// ─── Path encoding ──────────────────────────────────────────────────────────

#[test]
fn encode_path_quotes_each_segment_but_keeps_slashes() {
    assert_eq!(encode_path("a/b.md"), "a/b.md");
    assert_eq!(encode_path("a dir/my file.md"), "a%20dir/my%20file.md");
    assert_eq!(encode_path("p/ç.md"), "p/%C3%A7.md");
}

// ─── Deployments config ─────────────────────────────────────────────────────

const SAMPLE: &str = r#"
[prod]
url = "https://co-artelonga.fly.dev"
token_name = "prod"
default = true

[uat]
url = "https://co-artelonga-uat.fly.dev/"
token_name = "uat"
"#;

#[test]
fn resolve_deployment_defaults_to_marked_default() {
    let map = parse_deployments(SAMPLE).unwrap();
    let d = resolve_deployment(&map, None).unwrap();
    assert_eq!(d.key, "prod");
    assert_eq!(d.url, "https://co-artelonga.fly.dev");
    assert_eq!(d.token_name, "prod");
}

#[test]
fn resolve_deployment_honors_to_flag_and_trims_url() {
    let map = parse_deployments(SAMPLE).unwrap();
    let d = resolve_deployment(&map, Some("uat")).unwrap();
    assert_eq!(d.key, "uat");
    assert_eq!(d.url, "https://co-artelonga-uat.fly.dev"); // trailing slash trimmed
    assert_eq!(d.token_name, "uat");
}

#[test]
fn resolve_deployment_unknown_key_errors() {
    let map = parse_deployments(SAMPLE).unwrap();
    let err = resolve_deployment(&map, Some("staging")).unwrap_err();
    assert!(err.to_string().contains("unknown deployment"));
}

#[test]
fn token_name_defaults_to_deployment_key() {
    let map = parse_deployments("[prod]\nurl = \"https://x\"\ndefault = true\n").unwrap();
    let d = resolve_deployment(&map, None).unwrap();
    assert_eq!(d.token_name, "prod");
}

// ─── Baseline namespacing ───────────────────────────────────────────────────

#[test]
fn baselines_are_namespaced_per_deployment() {
    let _g = ENV_LOCK.lock().unwrap();
    let tmp = TempDir::new().unwrap();
    // SAFETY: guarded by ENV_LOCK so no other test reads CO_HOME concurrently.
    unsafe { std::env::set_var("CO_HOME", tmp.path()) };

    save_baseline("prod", "artelonga", "commitA").unwrap();
    save_baseline("uat", "artelonga", "commitB").unwrap();

    assert_eq!(
        read_baseline("prod", "artelonga").as_deref(),
        Some("commitA")
    );
    assert_eq!(
        read_baseline("uat", "artelonga").as_deref(),
        Some("commitB")
    );
    assert_eq!(read_baseline("prod", "other"), None);

    unsafe { std::env::remove_var("CO_HOME") };
}

// ─── Changelog rendering ────────────────────────────────────────────────────

#[test]
fn changelog_first_upload_has_no_commit_section() {
    let body = render_changelog("uni", Path::new("/src"), None, "cur123", "");
    assert!(body.contains("# Upload run — uni"));
    assert!(body.contains("baseline: <none>"));
    assert!(body.contains("First upload"));
    assert!(!body.contains("Commits since last upload"));
}

#[test]
fn changelog_delta_includes_commit_log() {
    let log = "abc 2026-06-13 10:00 — fix typo\ndef 2026-06-13 11:00 — add page\n";
    let body = render_changelog("uni", Path::new("/src"), Some("base9"), "cur9", log);
    assert!(body.contains("baseline: base9"));
    assert!(body.contains("Commits since last upload"));
    assert!(body.contains("fix typo"));
    assert!(body.contains("add page"));
}

// ─── Fake source / uploader ─────────────────────────────────────────────────

struct FakeSource {
    current: String,
    all: Vec<String>,
    changed: Vec<String>,
    valid: bool,
    log: String,
}

impl DeltaSource for FakeSource {
    fn current(&self) -> Result<String> {
        Ok(self.current.clone())
    }
    fn baseline_valid(&self, _: &str) -> Result<bool> {
        Ok(self.valid)
    }
    fn changed(&self, _: &str) -> Result<Vec<String>> {
        Ok(self.changed.clone())
    }
    fn all_files(&self) -> Result<Vec<String>> {
        Ok(self.all.clone())
    }
    fn log(&self, _: &str) -> Result<String> {
        Ok(self.log.clone())
    }
}

#[derive(Default)]
struct FakeUploader {
    uploaded: RefCell<Vec<String>>,
    events: RefCell<Vec<String>>,
}

impl Uploader for FakeUploader {
    fn verify(&self) -> Result<()> {
        Ok(())
    }
    fn upload(&self, path: &str, _content: &str) -> Result<()> {
        self.uploaded.borrow_mut().push(path.to_string());
        Ok(())
    }
    fn post_event(&self, path: &str, _markdown: &str) -> Result<()> {
        self.events.borrow_mut().push(path.to_string());
        Ok(())
    }
}

// ─── plan_files ─────────────────────────────────────────────────────────────

fn src(changed: &[&str], all: &[&str], valid: bool) -> FakeSource {
    FakeSource {
        current: "cur".to_string(),
        all: all.iter().map(|s| s.to_string()).collect(),
        changed: changed.iter().map(|s| s.to_string()).collect(),
        valid,
        log: "x 2026 — msg".to_string(),
    }
}

#[test]
fn plan_no_baseline_is_full_upload() {
    let s = src(&["a.md"], &["a.md", "b.md", "skip.txt"], true);
    let (files, full) = plan_files(&s, None, false, &FileFilter::new(None, None)).unwrap();
    assert!(full);
    assert_eq!(files, vec!["a.md", "b.md"]); // skip.txt filtered out
}

#[test]
fn plan_with_baseline_is_delta() {
    let s = src(&["b.md", "x.txt"], &["a.md", "b.md"], true);
    let (files, full) = plan_files(&s, Some("base"), false, &FileFilter::new(None, None)).unwrap();
    assert!(!full);
    assert_eq!(files, vec!["b.md"]);
}

#[test]
fn plan_invalid_baseline_falls_back_to_full() {
    let s = src(&["b.md"], &["a.md", "b.md"], false); // baseline lost
    let (files, full) = plan_files(&s, Some("gone"), false, &FileFilter::new(None, None)).unwrap();
    assert!(full);
    assert_eq!(files, vec!["a.md", "b.md"]);
}

#[test]
fn plan_full_flag_forces_full_even_with_baseline() {
    let s = src(&["b.md"], &["a.md", "b.md"], true);
    let (files, full) = plan_files(&s, Some("base"), true, &FileFilter::new(None, None)).unwrap();
    assert!(full);
    assert_eq!(files, vec!["a.md", "b.md"]);
}

// ─── push_with orchestration ────────────────────────────────────────────────

fn target_with_files(files: &[(&str, &str)]) -> (TempDir, Target) {
    let root = TempDir::new().unwrap();
    for (rel, body) in files {
        let p = root.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
    }
    let target = Target {
        universe: "artelonga".to_string(),
        root: root.path().to_path_buf(),
        filter: FileFilter::new(None, None),
    };
    (root, target)
}

fn deployment() -> Deployment {
    Deployment {
        key: "prod".to_string(),
        url: "https://example.test".to_string(),
        token_name: "prod".to_string(),
    }
}

#[test]
fn push_uploads_delta_saves_baseline_and_writes_changelog() {
    let _g = ENV_LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    unsafe { std::env::set_var("CO_HOME", home.path()) };
    save_baseline("prod", "artelonga", "oldbase").unwrap();

    let (_root, target) = target_with_files(&[("a.md", "# A"), ("b.md", "# B")]);
    let source = FakeSource {
        current: "newcommit".to_string(),
        all: vec!["a.md".into(), "b.md".into()],
        changed: vec!["a.md".into(), "b.md".into()],
        valid: true,
        log: "c1 2026 — edit a\n".to_string(),
    };
    let up = FakeUploader::default();
    let opts = PushOpts::default();

    push_with(&source, Some(&up), &deployment(), &target, &opts).unwrap();

    assert_eq!(up.uploaded.borrow().clone(), vec!["a.md", "b.md"]);
    // baseline advanced
    assert_eq!(
        read_baseline("prod", "artelonga").as_deref(),
        Some("newcommit")
    );
    // a changelog snippet was written
    let runs = home.path().join("sync-runs").join("prod");
    let n = std::fs::read_dir(&runs).unwrap().count();
    assert_eq!(n, 1, "exactly one changelog snippet");

    unsafe { std::env::remove_var("CO_HOME") };
}

#[test]
fn push_dry_run_uploads_nothing_and_keeps_baseline() {
    let _g = ENV_LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    unsafe { std::env::set_var("CO_HOME", home.path()) };
    save_baseline("prod", "artelonga", "oldbase").unwrap();

    let (_root, target) = target_with_files(&[("a.md", "# A")]);
    let source = src(&["a.md"], &["a.md"], true);
    let up = FakeUploader::default();
    let opts = PushOpts {
        dry_run: true,
        ..Default::default()
    };

    push_with(&source, Some(&up), &deployment(), &target, &opts).unwrap();

    assert!(up.uploaded.borrow().is_empty(), "dry-run uploads nothing");
    assert_eq!(
        read_baseline("prod", "artelonga").as_deref(),
        Some("oldbase")
    );

    unsafe { std::env::remove_var("CO_HOME") };
}

#[test]
fn push_changelog_flag_posts_event() {
    let _g = ENV_LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    unsafe { std::env::set_var("CO_HOME", home.path()) };
    save_baseline("prod", "artelonga", "oldbase").unwrap();

    let (_root, target) = target_with_files(&[("a.md", "# A")]);
    let source = src(&["a.md"], &["a.md"], true);
    let up = FakeUploader::default();
    let opts = PushOpts {
        push_changelog: true,
        ..Default::default()
    };

    push_with(&source, Some(&up), &deployment(), &target, &opts).unwrap();

    let events = up.events.borrow();
    assert_eq!(events.len(), 1);
    assert!(events[0].starts_with("events/sync-"));

    unsafe { std::env::remove_var("CO_HOME") };
}

#[test]
fn synthesize_event_has_sync_frontmatter() {
    let ev = synthesize_event("artelonga", 3, "log body");
    assert!(ev.contains("type: event"));
    assert!(ev.contains("kind: sync"));
    assert!(ev.contains("3 commit(s) synced"));
    assert!(ev.contains("log body"));
}

// ─── jj integration: dry-run plan == `jj diff --name-only` ──────────────────

fn jj_available() -> bool {
    Command::new("jj").arg("--version").output().is_ok()
}

fn jj_in(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("jj")
        .args(args)
        .current_dir(dir)
        .env("JJ_USER", "test")
        .env("JJ_EMAIL", "test@example.com")
        .output()
        .expect("run jj")
}

#[test]
fn dry_run_plan_matches_jj_diff_name_only() {
    if !jj_available() {
        eprintln!("skipping dry_run_plan_matches_jj_diff_name_only: jj not installed");
        return;
    }
    let repo = TempDir::new().unwrap();
    let root = repo.path();

    // Initialize a standalone jj repo and seed an `init` commit.
    assert!(jj_in(root, &["git", "init"]).status.success());
    std::fs::write(root.join("keep1.md"), "v1\n").unwrap();
    std::fs::create_dir_all(root.join("dir")).unwrap();
    std::fs::write(root.join("dir/keep2.md"), "k2\n").unwrap();
    std::fs::write(root.join("note.txt"), "txt\n").unwrap();
    assert!(jj_in(root, &["describe", "-m", "init"]).status.success());
    // Start a fresh working-copy commit on top of `init`.
    assert!(jj_in(root, &["new"]).status.success());

    // Baseline = the `init` commit id (parent of the new working copy).
    let base = String::from_utf8(
        jj_in(root, &["log", "-r", "@-", "--no-graph", "-T", "commit_id"]).stdout,
    )
    .unwrap()
    .lines()
    .next()
    .unwrap()
    .trim()
    .to_string();

    // Make changes in the working copy.
    std::fs::write(root.join("keep1.md"), "v2\n").unwrap();
    std::fs::write(root.join("new.md"), "new\n").unwrap();
    std::fs::write(root.join("ignore.txt"), "nope\n").unwrap();

    // Our engine's view of the delta (read-only jj ops need no user config).
    let source = JjSource::new(root.to_path_buf());
    let filter = FileFilter::new(None, None);
    let (planned, was_full) = plan_files(&source, Some(&base), false, &filter).unwrap();
    assert!(!was_full, "valid baseline → delta upload");

    // jj's own name-only diff, filtered the same way.
    let raw = String::from_utf8(
        jj_in(root, &["diff", "--from", &base, "--to", "@", "--name-only"]).stdout,
    )
    .unwrap();
    let mut expected: Vec<String> = raw
        .lines()
        .map(|l| l.trim().replace('\\', "/"))
        .filter(|l| !l.is_empty() && filter.matches(l))
        .collect();
    expected.sort();
    expected.dedup();

    assert_eq!(planned, expected);
    // Sanity: the markdown changes are present, the .txt is excluded.
    assert!(planned.contains(&"keep1.md".to_string()));
    assert!(planned.contains(&"new.md".to_string()));
    assert!(!planned.iter().any(|p| p.ends_with(".txt")));
}
