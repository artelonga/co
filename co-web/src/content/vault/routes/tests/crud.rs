use super::support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tempfile::tempdir;
use tower::ServiceExt;

// -----------------------------------------------------------------------
// CRUD cycle
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_vault_crud_cycle() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "test-vault");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    let content = "---\ntitle: Test Note\ntags: [rust, test]\ntype: note\n---\n\nHello vault!";

    // PUT — create file
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(content))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert_eq!(json["path"], "notes/hello.md");
    assert_eq!(json["frontmatter"]["title"], "Test Note");

    // GET — read file
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert_eq!(json["frontmatter"]["title"], "Test Note");
    assert!(
        json["tags"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("rust"))
    );

    // GET / — list files
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/test-vault/vault/")
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
            .any(|f| f["path"] == "notes/hello.md")
    );

    // POST — append
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("Appended line."))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET — verify append
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(json["content"].as_str().unwrap().contains("Appended line."));

    // POST /search
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/test-vault/vault/search")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"query":"vault"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let results: serde_json::Value =
        serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(!results.as_array().unwrap().is_empty());

    // DELETE — soft
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET after delete — 404
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/test-vault/vault/notes/hello.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// -----------------------------------------------------------------------
// PATCH targeted edits
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_vault_patch_frontmatter() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "patch-test");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    let content = "---\ntitle: Patch Me\ntags: [old]\ntype: note\n---\n\nBody.";
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/patch-test/vault/patch.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(content))
                .unwrap(),
        )
        .await
        .unwrap();

    // PATCH frontmatter — replace tags
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/universes/patch-test/vault/patch.md")
                .header(header::AUTHORIZATION, &bearer)
                .header("target-type", "frontmatter")
                .header("target", "tags")
                .header("operation", "replace")
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(r#"["new","updated"]"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/patch-test/vault/patch.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    let tags: Vec<&str> = json["frontmatter"]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(tags.contains(&"new"), "Expected 'new' in {tags:?}");
    assert!(tags.contains(&"updated"), "Expected 'updated' in {tags:?}");
    assert!(
        !tags.contains(&"old"),
        "Should not contain 'old' in {tags:?}"
    );
}

#[tokio::test]
async fn test_vault_patch_heading() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "heading-test");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    let content = "---\ntitle: Doc\ntype: note\n---\n\n## Introduction\n\nOld intro text.\n\n## Other\n\nOther content.";
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/heading-test/vault/doc.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(content))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/universes/heading-test/vault/doc.md")
                .header(header::AUTHORIZATION, &bearer)
                .header("target-type", "heading")
                .header("target", "## Introduction")
                .header("operation", "replace")
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("New intro text."))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/heading-test/vault/doc.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    let c = json["content"].as_str().unwrap();
    assert!(c.contains("New intro text."), "Expected new text in: {c}");
    assert!(
        !c.contains("Old intro text."),
        "Should not contain old: {c}"
    );
    assert!(c.contains("## Other"), "Should keep other sections: {c}");
}

// -----------------------------------------------------------------------
// Clipper format
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_vault_clipper_post() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "clipper-test");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    let clip_req = serde_json::json!({
        "content": "---\ntitle: \"Article Title\"\nsource: \"https://example.com/article\"\nauthor: \"Author Name\"\npublished: \"2026-04-06\"\ncreated: \"2026-04-06T13:00:00Z\"\ntags: [web-clip, topic]\n---\n\n# Article Title\n\nClipped content here..."
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/clipper-test/vault/clip")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(clip_req.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let result: serde_json::Value =
        serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(
        result["path"]
            .as_str()
            .unwrap()
            .starts_with("content/clips/"),
        "Path should be in content/clips/: {}",
        result["path"]
    );
    assert!(
        result["slug"].as_str().unwrap().contains("article-title"),
        "Slug should contain 'article-title': {}",
        result["slug"]
    );

    // Read back the clip and verify fields
    let path = result["path"].as_str().unwrap().to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/clipper-test/vault/{path}"))
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let file: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert_eq!(file["frontmatter"]["type"], "clip");
    assert_eq!(file["frontmatter"]["source"], "https://example.com/article");
    assert!(
        file["tags"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("web-clip"))
    );
}
