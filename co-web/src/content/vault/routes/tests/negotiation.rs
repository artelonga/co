//! CO-86 — Vault API content-negotiation: the same entry is served as JSON
//! (default), raw `text/markdown`, or the `.co` protobuf envelope, selected by
//! the `Accept` header.

use super::support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tempfile::tempdir;
use tower::ServiceExt;

const CONTENT: &str =
    "---\ntitle: Negotiated\ntype: note\ntags: [a, b]\n---\n\n# Olá\n\nCorpo do texto.\n";

async fn seed_and_put(dir: &std::path::Path, app: &axum::Router, bearer: &str) {
    let _ = dir;
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/neg/vault/notes/n.md")
                .header(header::AUTHORIZATION, bearer)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from(CONTENT))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn vault_get_negotiates_json_markdown_and_co() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "neg");
    let app = build_test_router(dir.path());
    let bearer = test_bearer();
    seed_and_put(dir.path(), &app, &bearer).await;

    // 1) Default (no Accept) → JSON VaultFile.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/neg/vault/notes/n.md")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.contains("application/json"),
        "default should be JSON, got {ct}"
    );
    let json: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert_eq!(json["frontmatter"]["title"], "Negotiated");

    // 2) Accept: text/markdown → raw markdown.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/neg/vault/notes/n.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::ACCEPT, "text/markdown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(ct.contains("text/markdown"), "got {ct}");
    let md = body_str(resp.into_body()).await;
    assert!(md.contains("# Olá"));
    assert!(md.contains("title: Negotiated"));

    // 3) Accept: application/vnd.co+protobuf → `.co` envelope bytes.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/neg/vault/notes/n.md")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::ACCEPT, co::co_format::MIME_TYPE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(ct.contains(co::co_format::MIME_TYPE), "got {ct}");

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(
        co::co_format::is_co_file(&bytes),
        "must carry .co magic bytes"
    );

    // The envelope decodes back to the same markdown, carrying the universe/path.
    let decoded = co::co_format::from_bytes(&bytes).unwrap();
    assert_eq!(decoded.universe_key, "neg");
    assert_eq!(decoded.entry_path, "notes/n.md");
    let rendered = co::co_format::to_markdown(&decoded).unwrap();
    assert!(rendered.contains("# Olá"));
}
