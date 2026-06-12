use super::super::*;
use super::support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tempfile::tempdir;
use tower::ServiceExt;

// -----------------------------------------------------------------------
// Integration: Obsidian plugin mock → vault API → verify SQLite state
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_obsidian_plugin_mock_flow() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "obs-test");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    // Simulate plugin writing a note
    let note = "---\ntitle: Plugin Note\ntype: note\ntags: [from-plugin]\n---\n\nPlugin body.";
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/obs-test/vault/notes/plugin-note.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(note))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "PUT should succeed");

    // Verify via vault listing — entry should appear
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/obs-test/vault/")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let files: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(
        files
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["path"] == "notes/plugin-note.md"),
        "Entry should appear in vault listing (SQLite indexed): {files}"
    );

    // Verify content preserved correctly
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/obs-test/vault/notes/plugin-note.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let file: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert_eq!(
        file["frontmatter"]["title"], "Plugin Note",
        "SQLite should have the correct title"
    );
    assert!(
        file["tags"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("from-plugin")),
        "Tags should be preserved in SQLite"
    );
}

// -----------------------------------------------------------------------
// Rate limiter unit test (no network)
// -----------------------------------------------------------------------

#[test]
fn test_rate_limiter_allows_up_to_60() {
    // Use a unique id per test run to avoid cross-test interference
    let id = format!("rate-test-{}", nanoid::nanoid!(12));
    for i in 0..60 {
        assert!(check_rate_limit(&id), "Request {i} should be within limit");
    }
    assert!(
        !check_rate_limit(&id),
        "61st request should be rate-limited"
    );
}
