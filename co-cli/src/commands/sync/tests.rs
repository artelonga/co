//! CO-51 sync engine tests.
//!
//! The diff/conflict/merge logic is exercised in-process against a mock
//! [`VaultClient`] (no network ports). One loopback test covers the real
//! reqwest path of [`HttpVaultClient`].

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::RwLock;

use super::*;
use tempfile::TempDir;

// ─── Mock Vault client ──────────────────────────────────────────────────────

#[derive(Default)]
struct MockState {
    /// path → (content, mtime_ms)
    files: BTreeMap<String, (String, i64)>,
    /// If set, `read(path)` errors — used to simulate a mid-sync crash.
    fail_read: Option<String>,
}

#[derive(Clone)]
struct MockClient {
    state: Rc<RwLock<MockState>>,
    writes: Rc<Cell<usize>>,
    deletes: Rc<Cell<usize>>,
}

impl MockClient {
    fn new() -> Self {
        Self {
            state: Rc::new(RwLock::new(MockState::default())),
            writes: Rc::new(Cell::new(0)),
            deletes: Rc::new(Cell::new(0)),
        }
    }

    fn set(&self, path: &str, content: &str, mtime_ms: i64) {
        self.state
            .write()
            .unwrap()
            .files
            .insert(path.to_string(), (content.to_string(), mtime_ms));
    }

    fn fail_read_on(&self, path: &str) {
        self.state.write().unwrap().fail_read = Some(path.to_string());
    }

    fn content(&self, path: &str) -> Option<String> {
        self.state
            .read()
            .unwrap()
            .files
            .get(path)
            .map(|(c, _)| c.clone())
    }
}

impl VaultClient for MockClient {
    fn list(&self) -> Result<Vec<RemoteFile>> {
        Ok(self
            .state
            .read()
            .unwrap()
            .files
            .iter()
            .map(|(path, (content, mtime))| RemoteFile {
                path: path.clone(),
                body_hash: comparable_hash(content),
                mtime_ms: *mtime,
            })
            .collect())
    }

    fn read(&self, path: &str) -> Result<String> {
        let st = self.state.read().unwrap();
        if st.fail_read.as_deref() == Some(path) {
            bail!("simulated read failure for {path}");
        }
        st.files
            .get(path)
            .map(|(c, _)| c.clone())
            .ok_or_else(|| anyhow::anyhow!("not found: {path}"))
    }

    fn write(&self, path: &str, content: &str) -> Result<()> {
        self.writes.set(self.writes.get() + 1);
        self.state
            .write()
            .unwrap()
            .files
            .insert(path.to_string(), (content.to_string(), 1_000));
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        self.deletes.set(self.deletes.get() + 1);
        self.state.write().unwrap().files.remove(path);
        Ok(())
    }
}

fn engine(client: MockClient, dir: &TempDir, strategy: Strategy) -> SyncEngine<MockClient> {
    SyncEngine::new(client, dir.path().join("uni"), strategy)
}

fn read_local(dir: &TempDir, path: &str) -> String {
    fs::read_to_string(dir.path().join("uni").join(path)).unwrap()
}

fn write_local_file(dir: &TempDir, path: &str, content: &str) {
    let p = dir.path().join("uni").join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, content).unwrap();
}

// ─── classify() ─────────────────────────────────────────────────────────────

#[test]
fn test_classify_local_new() {
    assert_eq!(classify(None, Some("h"), None), ChangeKind::LocalNew);
}

#[test]
fn test_classify_remote_new() {
    assert_eq!(classify(None, None, Some("h")), ChangeKind::RemoteNew);
}

#[test]
fn test_classify_unchanged() {
    let base = FileState {
        hash: "h".into(),
        mtime: None,
        remote_hash: "h".into(),
    };
    assert_eq!(
        classify(Some(&base), Some("h"), Some("h")),
        ChangeKind::Unchanged
    );
}

#[test]
fn test_classify_local_modified() {
    let base = FileState {
        hash: "h".into(),
        mtime: None,
        remote_hash: "h".into(),
    };
    assert_eq!(
        classify(Some(&base), Some("x"), Some("h")),
        ChangeKind::LocalModified
    );
}

#[test]
fn test_classify_remote_modified() {
    let base = FileState {
        hash: "h".into(),
        mtime: None,
        remote_hash: "h".into(),
    };
    assert_eq!(
        classify(Some(&base), Some("h"), Some("y")),
        ChangeKind::RemoteModified
    );
}

#[test]
fn test_classify_conflict() {
    let base = FileState {
        hash: "h".into(),
        mtime: None,
        remote_hash: "h".into(),
    };
    assert_eq!(
        classify(Some(&base), Some("x"), Some("y")),
        ChangeKind::Conflict
    );
}

#[test]
fn test_classify_remote_deleted() {
    let base = FileState {
        hash: "h".into(),
        mtime: None,
        remote_hash: "h".into(),
    };
    assert_eq!(
        classify(Some(&base), Some("h"), None),
        ChangeKind::RemoteDeleted
    );
    // local also changed → conflict, not a silent delete
    assert_eq!(classify(Some(&base), Some("x"), None), ChangeKind::Conflict);
}

#[test]
fn test_classify_local_deleted() {
    let base = FileState {
        hash: "h".into(),
        mtime: None,
        remote_hash: "h".into(),
    };
    assert_eq!(
        classify(Some(&base), None, Some("h")),
        ChangeKind::LocalDeleted
    );
}

// ─── merge3() ───────────────────────────────────────────────────────────────

#[test]
fn test_merge3_non_overlapping() {
    let base = "a\nb\nc\nd\ne\n";
    let local = "a\nB2\nc\nd\ne\n"; // changed line 2
    let remote = "a\nb\nc\nd\nE5\n"; // changed line 5
    let merged = merge3(base, local, remote).expect("non-overlapping merges");
    assert_eq!(merged, "a\nB2\nc\nd\nE5\n");
}

#[test]
fn test_merge3_overlapping_conflict() {
    let base = "a\nb\nc\n";
    let local = "a\nLOCAL\nc\n";
    let remote = "a\nREMOTE\nc\n";
    assert!(merge3(base, local, remote).is_err());
}

#[test]
fn test_merge3_identical_changes() {
    let base = "a\nb\nc\n";
    let same = "a\nX\nc\n";
    assert_eq!(merge3(base, same, same).unwrap(), "a\nX\nc\n");
}

#[test]
fn test_merge3_local_only_change() {
    let base = "a\nb\nc\n";
    let local = "a\nb\nc\nd\n"; // appended a line
    let remote = "a\nb\nc\n";
    assert_eq!(merge3(base, local, remote).unwrap(), "a\nb\nc\nd\n");
}

// ─── pull ───────────────────────────────────────────────────────────────────

#[test]
fn test_pull_downloads_all_entries() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    mock.set("content/sobre.md", "---\ntitle: Sobre\n---\nOlá\n", 10);
    mock.set("projects/CO/_project.md", "# CO\n", 20);

    let eng = engine(mock, &dir, Strategy::LastWriteWins);
    let summary = eng.pull().unwrap();

    assert_eq!(summary.pulled, 2);
    assert_eq!(
        read_local(&dir, "content/sobre.md"),
        "---\ntitle: Sobre\n---\nOlá\n"
    );
    assert_eq!(read_local(&dir, "projects/CO/_project.md"), "# CO\n");
    assert!(dir.path().join("uni/.co/sync.json").is_file());
}

#[test]
fn test_pull_twice_idempotent() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    mock.set("a.md", "alpha\n", 10);

    let eng = engine(mock, &dir, Strategy::LastWriteWins);
    let first = eng.pull().unwrap();
    assert_eq!(first.pulled, 1);

    let second = eng.pull().unwrap();
    assert_eq!(second.pulled, 0, "second pull is a no-op");
    assert_eq!(read_local(&dir, "a.md"), "alpha\n");
}

// ─── push ───────────────────────────────────────────────────────────────────

#[test]
fn test_push_uploads_changed_files() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    mock.set("a.md", "v1\n", 10);
    let eng = engine(mock.clone(), &dir, Strategy::LastWriteWins);
    eng.pull().unwrap();

    // Edit locally, then push.
    write_local_file(&dir, "a.md", "v2-local\n");
    let summary = eng.push().unwrap();

    assert_eq!(summary.pushed, 1);
    assert_eq!(mock.content("a.md").as_deref(), Some("v2-local\n"));
}

#[test]
fn test_push_noop_when_clean() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    mock.set("a.md", "v1\n", 10);
    let eng = engine(mock.clone(), &dir, Strategy::LastWriteWins);
    eng.pull().unwrap();
    let writes_before = mock.writes.get();

    let summary = eng.push().unwrap();
    assert_eq!(summary.pushed, 0);
    assert_eq!(
        mock.writes.get(),
        writes_before,
        "no PUTs when nothing changed"
    );
}

#[test]
fn test_push_new_local_file() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    let eng = engine(mock.clone(), &dir, Strategy::LastWriteWins);

    write_local_file(&dir, "novo.md", "brand new\n");
    let summary = eng.push().unwrap();
    assert_eq!(summary.pushed, 1);
    assert_eq!(mock.content("novo.md").as_deref(), Some("brand new\n"));
}

// ─── status ─────────────────────────────────────────────────────────────────

#[test]
fn test_status_accurate_diff() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    mock.set("a.md", "v1\n", 10);
    let eng = engine(mock.clone(), &dir, Strategy::LastWriteWins);
    eng.pull().unwrap();
    assert!(eng.status().unwrap().is_empty(), "clean after pull");

    write_local_file(&dir, "a.md", "v2\n");
    mock.set("b.md", "remote-only\n", 30);

    let changes = eng.status().unwrap();
    let by_path: BTreeMap<_, _> = changes.iter().map(|c| (c.path.as_str(), c.kind)).collect();
    assert_eq!(by_path.get("a.md"), Some(&ChangeKind::LocalModified));
    assert_eq!(by_path.get("b.md"), Some(&ChangeKind::RemoteNew));
}

// ─── conflict detection & resolution ────────────────────────────────────────

/// Set up a synced file, then diverge both sides. Returns the engine.
fn diverged(
    dir: &TempDir,
    mock: &MockClient,
    strategy: Strategy,
    remote_mtime: i64,
) -> SyncEngine<MockClient> {
    mock.set("c.md", "base\n", 10);
    let eng = engine(mock.clone(), dir, strategy);
    eng.pull().unwrap();
    // Both sides edit the file differently.
    write_local_file(dir, "c.md", "local change\n");
    mock.set("c.md", "remote change\n", remote_mtime);
    eng
}

#[test]
fn test_conflict_is_detected() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    let eng = diverged(&dir, &mock, Strategy::Manual, 20);
    let changes = eng.status().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, ChangeKind::Conflict);
}

#[test]
fn test_last_write_wins_local_newer() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    // remote mtime in the distant past → local (real now) wins.
    let eng = diverged(&dir, &mock, Strategy::LastWriteWins, 0);
    let summary = eng.push().unwrap();
    assert_eq!(summary.conflicts, 1);
    assert_eq!(mock.content("c.md").as_deref(), Some("local change\n"));
}

#[test]
fn test_last_write_wins_remote_newer() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    // remote mtime far in the future → remote wins.
    let eng = diverged(&dir, &mock, Strategy::LastWriteWins, i64::MAX);
    let summary = eng.pull().unwrap();
    assert_eq!(summary.conflicts, 1);
    assert_eq!(read_local(&dir, "c.md"), "remote change\n");
}

#[test]
fn test_local_wins() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    let eng = diverged(&dir, &mock, Strategy::LocalWins, i64::MAX);
    eng.push().unwrap();
    assert_eq!(mock.content("c.md").as_deref(), Some("local change\n"));
}

#[test]
fn test_remote_wins() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    let eng = diverged(&dir, &mock, Strategy::RemoteWins, 0);
    eng.pull().unwrap();
    assert_eq!(read_local(&dir, "c.md"), "remote change\n");
}

#[test]
fn test_manual_creates_artifacts() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    let eng = diverged(&dir, &mock, Strategy::Manual, 20);
    let summary = eng.push().unwrap();

    assert_eq!(summary.unresolved, vec!["c.md".to_string()]);
    let base = dir.path().join("uni/c.md");
    assert_eq!(
        fs::read_to_string(sibling_path(&base, "local")).unwrap(),
        "local change\n"
    );
    assert_eq!(
        fs::read_to_string(sibling_path(&base, "remote")).unwrap(),
        "remote change\n"
    );
    assert_eq!(
        fs::read_to_string(sibling_path(&base, "base")).unwrap(),
        "base\n"
    );
    // State NOT advanced — still conflicted on next run.
    assert_eq!(eng.status().unwrap()[0].kind, ChangeKind::Conflict);
}

#[test]
fn test_merge_strategy_non_overlapping() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    mock.set("m.md", "a\nb\nc\nd\ne\n", 10);
    let eng = engine(mock.clone(), &dir, Strategy::Merge);
    eng.pull().unwrap();

    write_local_file(&dir, "m.md", "a\nB2\nc\nd\ne\n"); // local edits line 2
    mock.set("m.md", "a\nb\nc\nd\nE5\n", 20); // remote edits line 5

    let summary = eng.push().unwrap();
    assert_eq!(summary.conflicts, 1);
    assert!(
        summary.unresolved.is_empty(),
        "auto-merged, no manual fallback"
    );
    assert_eq!(mock.content("m.md").as_deref(), Some("a\nB2\nc\nd\nE5\n"));
    assert_eq!(read_local(&dir, "m.md"), "a\nB2\nc\nd\nE5\n");
}

#[test]
fn test_merge_strategy_overlap_falls_back_to_manual() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    let eng = diverged(&dir, &mock, Strategy::Merge, 20); // both edit same line
    let summary = eng.push().unwrap();
    assert_eq!(summary.unresolved, vec!["c.md".to_string()]);
    let base = dir.path().join("uni/c.md");
    assert!(sibling_path(&base, "remote").is_file());
}

// ─── resolve ────────────────────────────────────────────────────────────────

#[test]
fn test_resolve_manual_finalizes() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    let eng = diverged(&dir, &mock, Strategy::Manual, 20);
    eng.push().unwrap();

    // User merges by hand and saves the file.
    write_local_file(&dir, "c.md", "merged by hand\n");
    eng.resolve_manual("c.md").unwrap();

    assert_eq!(mock.content("c.md").as_deref(), Some("merged by hand\n"));
    let base = dir.path().join("uni/c.md");
    assert!(!sibling_path(&base, "local").exists());
    assert!(!sibling_path(&base, "remote").exists());
    assert!(!sibling_path(&base, "base").exists());
    assert!(eng.status().unwrap().is_empty(), "in sync after resolve");
}

// ─── crash recovery ─────────────────────────────────────────────────────────

#[test]
fn test_crash_recovery_partial_pull() {
    let dir = TempDir::new().unwrap();
    let mock = MockClient::new();
    mock.set("a.md", "alpha\n", 10);
    mock.set("z.md", "zeta\n", 20);
    mock.fail_read_on("z.md"); // crash while pulling the second file

    let eng = engine(mock.clone(), &dir, Strategy::LastWriteWins);
    let err = eng.pull();
    assert!(err.is_err(), "pull surfaces the failure");

    // sync.json must be valid and reflect only the file that succeeded.
    let state = SyncState::load(&dir.path().join("uni/.co/sync.json")).unwrap();
    assert!(state.files.contains_key("a.md"));
    assert!(!state.files.contains_key("z.md"));

    // Recovery: clear the fault and re-run — the rest syncs, no duplication.
    mock.state.write().unwrap().fail_read = None;
    let summary = eng.pull().unwrap();
    assert_eq!(summary.pulled, 1);
    assert_eq!(read_local(&dir, "z.md"), "zeta\n");
}

// ─── lockfile ───────────────────────────────────────────────────────────────

#[test]
fn test_lockfile_prevents_concurrent_sync() {
    let dir = TempDir::new().unwrap();
    let co_dir = dir.path().join("uni/.co");
    let _held = SyncLock::acquire(&co_dir).unwrap();
    let second = SyncLock::acquire(&co_dir);
    assert!(second.is_err(), "second lock acquisition must fail");
}

#[test]
fn test_lockfile_released_on_drop() {
    let dir = TempDir::new().unwrap();
    let co_dir = dir.path().join("uni/.co");
    {
        let _held = SyncLock::acquire(&co_dir).unwrap();
    }
    // Lock dropped → can re-acquire.
    assert!(SyncLock::acquire(&co_dir).is_ok());
}

// ─── strategy parsing ───────────────────────────────────────────────────────

#[test]
fn test_strategy_parse() {
    assert_eq!(
        Strategy::parse("last-write-wins").unwrap(),
        Strategy::LastWriteWins
    );
    assert_eq!(Strategy::parse("local-wins").unwrap(), Strategy::LocalWins);
    assert_eq!(
        Strategy::parse("remote-wins").unwrap(),
        Strategy::RemoteWins
    );
    assert_eq!(Strategy::parse("manual").unwrap(), Strategy::Manual);
    assert_eq!(Strategy::parse("merge").unwrap(), Strategy::Merge);
    assert!(Strategy::parse("nonsense").is_err());
}

// ─── HTTP client (loopback mock server) ─────────────────────────────────────

/// CO-51: exercise the real reqwest `HttpVaultClient` against an in-process
/// loopback HTTP server (bound to 127.0.0.1 only, deterministic, no external
/// network). Mirrors the accepted pattern in `auth::ua_tests`.
#[test]
fn test_http_client_against_loopback_server() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let server = std::thread::spawn(move || {
        // 1) GET /vault/  → listing
        // 2) GET /vault/a.md → file content
        // 3) PUT /vault/a.md → 200
        for expected in 0..3 {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = match expected {
                0 => {
                    assert!(req.starts_with("GET "), "first call is the listing: {req}");
                    assert!(
                        req.to_lowercase().contains("authorization: bearer tok-xyz"),
                        "bearer token forwarded: {req}"
                    );
                    r#"[{"path":"a.md","body_hash":"abc","stat":{"mtime":1234}}]"#.to_string()
                }
                1 => {
                    assert!(req.starts_with("GET "));
                    r#"{"path":"a.md","content":"hello\n","frontmatter":{},"tags":[],"stat":{"mtime":1}}"#
                        .to_string()
                }
                _ => {
                    assert!(req.starts_with("PUT "), "third call writes: {req}");
                    assert!(req.contains("hello-edited"), "body forwarded: {req}");
                    "{}".to_string()
                }
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).unwrap();
        }
    });

    let client = HttpVaultClient::new(&format!("http://{addr}"), "yuri", "tok-xyz");

    let listing = client.list().unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].path, "a.md");
    assert_eq!(listing[0].body_hash, "abc");
    assert_eq!(listing[0].mtime_ms, 1234);

    let content = client.read("a.md").unwrap();
    assert_eq!(content, "hello\n");

    client.write("a.md", "hello-edited\n").unwrap();

    server.join().unwrap();
}
