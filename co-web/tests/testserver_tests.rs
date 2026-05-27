//! CO-300: Integration tests using `TestServer` — exercises the real `co serve` binary.
//!
//! These tests replace the in-process `build_test_router` pattern with a subprocess-based
//! server so that middleware ordering, HTTP semantics, and serialization edge cases are
//! exercised through the full code path.

mod testkit;

use serde_json::json;

// ─── 1. Health check ──────────────────────────────────────────────────────────

/// Server must respond to GET /health with 200.
/// The <2s startup requirement from CO-300 applies to release builds; debug builds
/// initialize slower (SQLite migrations, seeding) and are not held to that threshold.
#[tokio::test]
async fn test_ts_health_check() {
    let server = testkit::TestServer::start().await;

    let resp = server
        .client()
        .get(server.url("/api/health"))
        .send()
        .await
        .expect("GET /api/health");

    assert!(
        resp.status().is_success(),
        "/api/health returned {}",
        resp.status()
    );

    let body: serde_json::Value = resp.json().await.expect("parse health JSON");
    assert_eq!(body["status"], "ok", "health status must be ok");
}

// ─── 2. Template universe — public read ──────────────────────────────────────

/// Anonymous GET /api/v1/universes/template/projects must return the CO project.
/// Mirrors `test_template_projects_public` from template_tests.rs via real binary.
#[tokio::test]
async fn test_ts_template_projects_public() {
    let server = testkit::TestServer::start().await;

    let resp = server
        .client()
        .get(server.url("/api/v1/universes/template/projects"))
        .send()
        .await
        .expect("GET template/projects");

    assert_eq!(resp.status(), 200, "expected 200; got {}", resp.status());

    let projects: Vec<serde_json::Value> = resp.json().await.expect("parse projects");
    assert_eq!(projects.len(), 1, "expected exactly one template project");
    assert_eq!(projects[0]["key"], "CO", "expected project key CO");
}

// ─── 3. Write to template — forbidden ────────────────────────────────────────

/// Authenticated POST to a template project must return 403 Forbidden.
/// Mirrors `test_write_to_template_forbidden` from template_tests.rs via real binary.
#[tokio::test]
async fn test_ts_write_to_template_forbidden() {
    let server = testkit::TestServer::start().await;

    let resp = server
        .client()
        .post(server.url("/api/projects/CO/tasks"))
        .header("Authorization", server.bearer())
        .header("Content-Type", "application/json")
        .body(json!({"title": "Should be blocked"}).to_string())
        .send()
        .await
        .expect("POST to template project");

    assert_eq!(
        resp.status(),
        403,
        "expected 403 Forbidden for write to template project; got {}",
        resp.status()
    );
}

// ─── 4. Clone template ───────────────────────────────────────────────────────

/// Cloning the template universe returns 201 and the new universe key.
/// Mirrors `test_clone_template` from template_tests.rs via real binary.
#[tokio::test]
async fn test_ts_clone_template() {
    let server = testkit::TestServer::start().await;

    let resp = server
        .client()
        .post(server.url("/api/v1/universes/template/clone"))
        .header("Content-Type", "application/json")
        .body(
            json!({
                "name": "Alice Universe",
                "key": "alice",
                "description": ""
            })
            .to_string(),
        )
        .send()
        .await
        .expect("POST clone");

    assert_eq!(
        resp.status(),
        201,
        "expected 201 Created; got {}",
        resp.status()
    );

    let universe: serde_json::Value = resp.json().await.expect("parse universe");
    assert_eq!(universe["key"], "alice", "unexpected universe key");
    assert_eq!(
        universe["is_template"], false,
        "cloned universe must not be a template"
    );
}

// ─── 5. Vault write-then-read ─────────────────────────────────────────────────

/// Vault PUT immediately visible in GET /entries (pipeline coherence).
/// Mirrors `vault_write_appears_in_list_immediately` from sync_pipeline_tests.rs via real binary.
/// Also verifies Cache-Control: no-store on the list response.
#[tokio::test]
async fn test_ts_vault_write_and_read() {
    let seed_sql = "
        INSERT OR IGNORE INTO users
            (id, email, display_name, created_at)
            VALUES ('test-user', 'test@example.com', 'Test User', '2026-01-01');
        INSERT OR IGNORE INTO universes
            (key, name, description, owner_id, created_at, is_public, visibility)
            VALUES ('vault-uni', 'Vault Test', '', 'test-user', '2026-01-01', 1, 'public-subscribable');
    "
    .to_string();

    let server = testkit::TestServer::start_with(testkit::TestServerConfig {
        seed_sql: Some(seed_sql),
        ..Default::default()
    })
    .await;

    let client = server.client();

    // Write a new entry via the Vault API.
    let put = client
        .put(server.url("/api/v1/universes/vault-uni/vault/tasks/ts-task.md"))
        .header("Authorization", server.bearer())
        .body("---\ntype: task\ntitle: TestServer task\nstatus: todo\n---\nBody.")
        .send()
        .await
        .expect("vault PUT");

    assert!(
        put.status().is_success(),
        "vault PUT must succeed; got {}",
        put.status()
    );

    // List entries immediately — must reflect the write.
    let list = client
        .get(server.url("/api/v1/universes/vault-uni/entries"))
        .header("Authorization", server.bearer())
        .send()
        .await
        .expect("GET entries");

    assert_eq!(list.status(), 200, "entries list must return 200");

    let cc = list
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(
        cc, "no-store",
        "list entries must return Cache-Control: no-store; got '{cc}'"
    );

    let body: serde_json::Value = list.json().await.expect("parse entries");
    let entries = body["entries"].as_array().expect("entries array");
    let found = entries
        .iter()
        .any(|e| e["path"].as_str() == Some("tasks/ts-task.md"));
    assert!(
        found,
        "newly written entry must appear in list; entries = {entries:?}"
    );
}

// ─── 6. Agent session POST + GET ─────────────────────────────────────────────

/// POST /api/v1/agent/sessions creates a session; GET retrieves it.
/// Exercises the agent session route through the real binary (no mocked auth middleware).
#[tokio::test]
async fn test_ts_agent_session_post_and_get() {
    let server = testkit::TestServer::start().await;
    let client = server.client();

    // POST a session record.
    let post = client
        .post(server.url("/api/v1/agent/sessions"))
        .header("Authorization", server.bearer())
        .header("Content-Type", "application/json")
        .body(
            json!({
                "task_id": "CO-300",
                "universe_key": "template",
                "started_at": 1_700_000_000_i64,
                "finished_at": 1_700_000_060_i64,
                "duration_ms": 60_000_i64,
                "exit_code": 0
            })
            .to_string(),
        )
        .send()
        .await
        .expect("POST agent session");

    assert_eq!(
        post.status(),
        201,
        "POST /api/v1/agent/sessions must return 201; got {}",
        post.status()
    );

    let created: serde_json::Value = post.json().await.expect("parse POST response");
    assert!(
        created["id"].as_i64().is_some(),
        "response must include an integer id"
    );

    // GET the session back.
    let get = client
        .get(server.url("/api/v1/agent/sessions?task_id=CO-300"))
        .send()
        .await
        .expect("GET agent sessions");

    assert_eq!(
        get.status(),
        200,
        "GET /api/v1/agent/sessions must return 200"
    );

    let list: serde_json::Value = get.json().await.expect("parse GET response");
    let total = list["total"].as_u64().unwrap_or(0);
    assert!(total >= 1, "expected at least 1 session; got total={total}");

    let sessions = list["sessions"].as_array().expect("sessions array");
    let found = sessions
        .iter()
        .any(|s| s["task_id"].as_str() == Some("CO-300"));
    assert!(
        found,
        "created session must appear in list; sessions = {sessions:?}"
    );
}
