//! CO-300: TestServer — spawns a real `co serve` process for integration tests.
//!
//! Each test file that wants to use TestServer adds `mod testkit;` at the top.
//!
//! ```rust,no_run
//! mod testkit;
//!
//! #[tokio::test]
//! async fn my_test() {
//!     let server = testkit::TestServer::start().await;
//!     let resp = server.client().get(server.url("/health")).send().await.unwrap();
//!     assert!(resp.status().is_success());
//! }
//! ```

use std::path::PathBuf;
use std::process::{Child, Stdio};
use tempfile::TempDir;
use tokio::time::{Duration, Instant};

// ─── Config ──────────────────────────────────────────────────────────────────

pub struct TestServerConfig {
    /// HMAC secret used to sign and verify HS256 JWTs.
    /// The test [`TestServer::bearer`] method signs with this same value.
    pub jwt_secret: String,
    /// Optional SQL executed by the server on first boot (seed.sql mechanism).
    /// Use this to pre-insert users, universes, or tasks before tests run.
    pub seed_sql: Option<String>,
}

impl Default for TestServerConfig {
    fn default() -> Self {
        Self {
            jwt_secret: "dev-secret-change-me".to_string(),
            seed_sql: None,
        }
    }
}

// ─── TestServer ───────────────────────────────────────────────────────────────

/// A real `co serve` subprocess bound to an ephemeral port on 127.0.0.1.
///
/// The server is shut down and its data directory cleaned up when `TestServer`
/// is dropped (or when [`TestServer::shutdown`] is called explicitly).
pub struct TestServer {
    /// `http://127.0.0.1:<port>` — prepend with [`TestServer::url`].
    pub base_url: String,
    _data_dir: TempDir,
    process: Option<Child>,
}

impl TestServer {
    /// Start a server with default config (dev JWT secret, no seed SQL).
    pub async fn start() -> Self {
        Self::start_with(TestServerConfig::default()).await
    }

    /// Start a server with custom config.
    pub async fn start_with(config: TestServerConfig) -> Self {
        let data_dir = tempfile::tempdir().expect("tempdir");

        if let Some(sql) = &config.seed_sql {
            std::fs::write(data_dir.path().join("seed.sql"), sql).expect("write seed.sql");
        }

        let port = free_port();
        let binary = ensure_co_binary();

        // Use a per-test game DB so concurrent test instances don't share
        // (and contend on) the same SQLite file.
        let game_db = data_dir
            .path()
            .join("game.db")
            .to_string_lossy()
            .into_owned();

        let child = std::process::Command::new(&binary)
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--data-dir",
                data_dir.path().to_str().expect("data_dir utf8"),
            ])
            .env("JWT_SECRET", &config.jwt_secret)
            .env("GAME_DB_PATH", &game_db)
            .env("RUST_LOG", "error")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn co serve: {e} — binary: {binary:?}"));

        let base_url = format!("http://127.0.0.1:{port}");
        wait_ready(&base_url).await;

        TestServer {
            base_url,
            _data_dir: data_dir,
            process: Some(child),
        }
    }

    /// A `reqwest::Client` with a cookie jar (useful for session-cookie auth flows).
    pub fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .expect("reqwest client")
    }

    /// Build a full URL by appending `path` to the server's base URL.
    ///
    /// ```
    /// server.url("/health")   // → "http://127.0.0.1:<port>/health"
    /// ```
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// A `Bearer <token>` value signed with the server's JWT secret.
    /// The subject is `"test-user"`, email `"test@example.com"`, tier `"player"`.
    pub fn bearer(&self) -> String {
        let (token, _) = co_web::auth::sign_jwt(
            "test-user",
            "test@example.com",
            "player",
            "dev-secret-change-me",
        )
        .expect("sign_jwt");
        format!("Bearer {token}")
    }

    /// Gracefully shut down the server and clean up the data directory.
    /// Calling this is optional — `Drop` will kill the process regardless.
    #[allow(dead_code)]
    pub async fn shutdown(mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ─── Internals ────────────────────────────────────────────────────────────────

/// Find a free TCP port on 127.0.0.1 by binding to port 0.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port 0");
    listener.local_addr().expect("local_addr").port()
}

/// Locate the `co` binary in the same target directory as the running test.
///
/// Integration test binaries live in `target/{debug,release}/deps/`; the `co`
/// binary lives one level up in `target/{debug,release}/`. The function also
/// triggers `cargo build -p co-cli` if the binary is missing (e.g. when only
/// `cargo test -p co-web` was invoked without a full workspace build first).
fn ensure_co_binary() -> PathBuf {
    let path = candidate_binary_path();
    if !path.exists() {
        let exe = std::env::current_exe().expect("current_exe");
        let release = exe.components().any(|c| c.as_os_str() == "release");

        let mut cmd = std::process::Command::new("cargo");
        cmd.args(["build", "-p", "co-cli", "--quiet"]);
        if release {
            cmd.arg("--release");
        }
        let status = cmd.status().expect("cargo build");
        assert!(status.success(), "cargo build -p co-cli failed");
    }
    assert!(
        path.exists(),
        "co binary not found at {path:?} after build attempt"
    );
    path
}

fn candidate_binary_path() -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    // test binary:  target/{debug,release}/deps/<name>-<hash>
    // co binary:    target/{debug,release}/co
    let target_profile_dir = exe
        .parent() // deps/
        .and_then(|p| p.parent()) // debug/ or release/
        .expect("resolve target dir");

    #[cfg(windows)]
    let name = "co.exe";
    #[cfg(not(windows))]
    let name = "co";

    target_profile_dir.join(name)
}

/// Poll `GET <base_url>/api/health` until it returns 2xx or the deadline expires.
async fn wait_ready(base_url: &str) {
    let client = reqwest::Client::new();
    let url = format!("{base_url}/api/health");
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        assert!(
            Instant::now() < deadline,
            "co server did not become ready within 30 s at {base_url}"
        );
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
