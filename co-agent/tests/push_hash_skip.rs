//! CO-267: integration test for `co-sync push` hash-skip logic.
//!
//! Spins up an in-process mock Vault API, runs the push logic twice, and
//! asserts that the second run skips unchanged files. All state is in-memory
//! or in a tempdir — no real network ports are opened (uses `tower::ServiceExt`
//! with an in-process router).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::put;
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Helpers matching the push logic from co-sync.rs
// ---------------------------------------------------------------------------

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn is_unchanged(cache: &HashMap<String, String>, cache_key: &str, content: &[u8]) -> bool {
    cache
        .get(cache_key)
        .is_some_and(|h| *h == sha256_hex(content))
}

fn glob_match(pattern: &str, name: &str) -> bool {
    glob_match_inner(
        &mut pattern.chars().peekable(),
        &mut name.chars().peekable(),
    )
}

fn glob_match_inner(
    p: &mut std::iter::Peekable<std::str::Chars<'_>>,
    n: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> bool {
    loop {
        match p.peek() {
            None => return n.peek().is_none(),
            Some('*') => {
                p.next();
                if p.peek().is_none() {
                    return true;
                }
                let rest_p: String = p.clone().collect();
                let rest_n: String = n.clone().collect();
                for i in 0..=rest_n.len() {
                    let mut pp = rest_p.chars().peekable();
                    let mut nn = rest_n[i..].chars().peekable();
                    if glob_match_inner(&mut pp, &mut nn) {
                        return true;
                    }
                }
                return false;
            }
            Some('?') => {
                p.next();
                if n.next().is_none() {
                    return false;
                }
            }
            Some(&pc) => {
                p.next();
                match n.next() {
                    Some(nc) if nc == pc => {}
                    _ => return false,
                }
            }
        }
    }
}

fn matches_patterns(name: &str, patterns: &[&str]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| glob_match(p, name))
}

fn should_include(name: &str, include: &[&str], exclude: &[&str]) -> bool {
    if !exclude.is_empty() && matches_patterns(name, exclude) {
        return false;
    }
    matches_patterns(name, include)
}

// ---------------------------------------------------------------------------
// Mock Vault API (in-process, no listening socket)
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MockVault {
    /// Counts PUT calls per path.
    put_counts: Arc<Mutex<HashMap<String, usize>>>,
}

async fn handle_put(
    State(vault): State<MockVault>,
    Path((slug, path)): Path<(String, String)>,
    body: Body,
) -> Response {
    // consume body to be realistic
    let _ = body.collect().await;
    let key = format!("{slug}/{path}");
    *vault.put_counts.lock().unwrap().entry(key).or_insert(0) += 1;
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::empty())
        .unwrap()
}

fn mock_router() -> (Router, Arc<Mutex<HashMap<String, usize>>>) {
    let vault = MockVault::default();
    let counts = Arc::clone(&vault.put_counts);
    let router = Router::new()
        .route("/api/v1/universes/{slug}/vault/{*path}", put(handle_put))
        .with_state(vault);
    (router, counts)
}

// ---------------------------------------------------------------------------
// Simulate the push logic (inline, avoids needing to call binary)
// ---------------------------------------------------------------------------

/// Push files from `source_dir` that match include/exclude and are not cached.
/// Returns (pushed, skipped).
async fn simulate_push(
    source_dir: &PathBuf,
    slug: &str,
    include: &[&str],
    exclude: &[&str],
    cache: &mut HashMap<String, String>,
    router: &Router,
) -> (usize, usize) {
    let mut pushed = 0;
    let mut skipped = 0;

    let entries = walkdir_files(source_dir);
    for (abs_path, rel_path) in entries {
        let name = abs_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !should_include(name, include, exclude) {
            continue;
        }
        let content = std::fs::read(&abs_path).unwrap();
        let cache_key = format!("{slug}/{rel_path}");
        if is_unchanged(cache, &cache_key, &content) {
            skipped += 1;
            continue;
        }
        let uri = format!("/api/v1/universes/{slug}/vault/{rel_path}");
        let req = Request::builder()
            .method("PUT")
            .uri(&uri)
            .header("Authorization", "Bearer test-token")
            .body(Body::from(content.clone()))
            .unwrap();
        let resp = router.clone().oneshot(req).await.unwrap();
        assert!(resp.status().is_success(), "PUT {uri} failed");
        cache.insert(cache_key, sha256_hex(&content));
        pushed += 1;
    }
    (pushed, skipped)
}

fn walkdir_files(dir: &PathBuf) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    fn recurse(root: &PathBuf, dir: &PathBuf, out: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                recurse(root, &path, out);
            } else if path.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    out.push((path, rel_str));
                }
            }
        }
    }
    recurse(dir, dir, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn push_uploads_all_files_on_first_run() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("work");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("CO-1.md"), b"task 1").unwrap();
    std::fs::write(src.join("CO-2.md"), b"task 2").unwrap();
    std::fs::write(src.join("CLAUDE.md"), b"excluded").unwrap();

    let (router, counts) = mock_router();
    let mut cache = HashMap::new();

    let (pushed, skipped) =
        simulate_push(&src, "co", &["*.md"], &["CLAUDE.md"], &mut cache, &router).await;

    assert_eq!(pushed, 2, "should upload CO-1.md + CO-2.md");
    assert_eq!(skipped, 0);
    let counts = counts.lock().unwrap();
    assert_eq!(counts.get("co/CO-1.md"), Some(&1));
    assert_eq!(counts.get("co/CO-2.md"), Some(&1));
    assert_eq!(
        counts.get("co/CLAUDE.md"),
        None,
        "CLAUDE.md must be excluded"
    );
}

#[tokio::test]
async fn push_skips_unchanged_files_on_second_run() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("work");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("CO-1.md"), b"task 1").unwrap();
    std::fs::write(src.join("CO-2.md"), b"task 2").unwrap();

    let (router, counts) = mock_router();
    let mut cache = HashMap::new();

    // First push — both files uploaded
    let (pushed1, skipped1) = simulate_push(&src, "co", &["*.md"], &[], &mut cache, &router).await;
    assert_eq!(pushed1, 2);
    assert_eq!(skipped1, 0);

    // Second push — same content → both skipped
    let (pushed2, skipped2) = simulate_push(&src, "co", &["*.md"], &[], &mut cache, &router).await;
    assert_eq!(pushed2, 0, "unchanged files must be skipped");
    assert_eq!(skipped2, 2);

    // PUT should still have been called exactly once per file
    let counts = counts.lock().unwrap();
    assert_eq!(counts.get("co/CO-1.md"), Some(&1));
    assert_eq!(counts.get("co/CO-2.md"), Some(&1));
}

#[tokio::test]
async fn push_reuploads_changed_files() {
    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("work");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("CO-1.md"), b"original").unwrap();

    let (router, counts) = mock_router();
    let mut cache = HashMap::new();

    // First push
    simulate_push(&src, "co", &["*.md"], &[], &mut cache, &router).await;

    // Modify the file
    std::fs::write(src.join("CO-1.md"), b"updated content").unwrap();

    // Second push — changed file must be re-uploaded
    let (pushed2, skipped2) = simulate_push(&src, "co", &["*.md"], &[], &mut cache, &router).await;
    assert_eq!(pushed2, 1, "changed file must be re-uploaded");
    assert_eq!(skipped2, 0);

    let counts = counts.lock().unwrap();
    assert_eq!(counts.get("co/CO-1.md"), Some(&2));
}

#[test]
fn glob_match_star() {
    assert!(glob_match("*.md", "CO-1.md"));
    assert!(glob_match("*.md", "CHANGELOG.md"));
    assert!(!glob_match("*.md", "CLAUDE.yaml"));
    assert!(!glob_match("*.md", "foo"));
}

#[test]
fn glob_match_exact() {
    assert!(glob_match("CLAUDE.md", "CLAUDE.md"));
    assert!(!glob_match("CLAUDE.md", "claude.md"));
    assert!(!glob_match("CLAUDE.md", "XCLAUDE.md"));
}

#[test]
fn should_include_respects_exclude_over_include() {
    // CLAUDE.md matches *.md (include) but also CLAUDE.md (exclude) → excluded
    assert!(!should_include("CLAUDE.md", &["*.md"], &["CLAUDE.md"]));
    // CO-1.md matches *.md (include) and not CLAUDE.md (exclude) → included
    assert!(should_include("CO-1.md", &["*.md"], &["CLAUDE.md"]));
    // .gitkeep matches nothing in include *.md → excluded
    assert!(!should_include(".gitkeep", &["*.md"], &[]));
}
