use super::super::*;
use super::support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tempfile::tempdir;
use tower::ServiceExt;

// -----------------------------------------------------------------------
// CO-418 e2e: publish imported entry → conventional commit + both
// provenance fields resolvable as typed relations; re-publish is a no-op.
// -----------------------------------------------------------------------

/// Seed an imported entry (CO-417 shape) directly into the DB as an owner
/// draft, then return its path. Mirrors what the source adapter writes.
async fn seed_imported_entry(app: &axum::Router, dir: &std::path::Path) -> String {
    let path = "imported/intro.md".to_string();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{SLUG}/entries"))
                .header(header::AUTHORIZATION, owner_bearer(dir))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "path": path,
                        "frontmatter": {
                            "type": "page",
                            "title": "Intro",
                            "source": "github:yurisugano/SensorySpeech@cafe1234",
                            "source_path": "docs/intro.md",
                            "source_kind": "github",
                        },
                        "body": "# Intro\n\nImported content.",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "seed imported entry should 201"
    );
    path
}

async fn get_entry_json(app: &axum::Router, dir: &std::path::Path, path: &str) -> JsonValue {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/universes/{SLUG}/entries/{path}"))
                .header(header::AUTHORIZATION, owner_bearer(dir))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp.into_body()).await
}

async fn publish_req(
    app: &axum::Router,
    dir: &std::path::Path,
    payload: serde_json::Value,
) -> (StatusCode, JsonValue) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{SLUG}/publish"))
                .header(header::AUTHORIZATION, owner_bearer(dir))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    (status, body_json(resp.into_body()).await)
}

#[tokio::test]
async fn test_publish_produces_conventional_commit_and_traceback() {
    let dir = tempdir().unwrap();
    seed_public_universe(dir.path());
    let app = build_test_router(dir.path());
    let path = seed_imported_entry(&app, dir.path()).await;

    // Publish, requesting via CO-419, as a feat(pipeline).
    let (status, out) = publish_req(
        &app,
        dir.path(),
        json!({
            "path": path,
            "requested_by": "CO-419",
            "commit_type": "feat",
            "commit_scope": "pipeline",
            "commit_subject": "publish SensorySpeech intro",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // (a) conventional commit + semver intent.
    assert_eq!(
        out["commit_message"], "feat(pipeline): publish SensorySpeech intro",
        "conventional commit message"
    );
    assert_eq!(out["semver_bump"], "minor", "feat ⇒ minor");
    assert_eq!(out["published"], true);
    assert_eq!(out["review_status"], "published");

    // (b) both provenance fields resolvable: stamped in frontmatter AND
    // surfaced as typed relations (origin → source, requested_by → task).
    // `EntryWithRelations` flattens the entry, so frontmatter is top-level.
    let entry = get_entry_json(&app, dir.path(), &path).await;
    let fm = &entry["frontmatter"];
    assert_eq!(fm["source"], "github:yurisugano/SensorySpeech@cafe1234");
    assert_eq!(fm["requested_by"], "CO-419");
    assert_eq!(
        fm["published_commit"],
        "feat(pipeline): publish SensorySpeech intro"
    );

    let relations = entry["relations"].as_array().expect("relations array");
    let origin = relations
        .iter()
        .find(|r| r["relation_type"] == "origin")
        .expect("origin relation");
    assert_eq!(
        origin["to_path"],
        "github:yurisugano/SensorySpeech@cafe1234"
    );
    let req = relations
        .iter()
        .find(|r| r["relation_type"] == "requested_by")
        .expect("requested_by relation");
    assert_eq!(req["to_path"], "CO-419");

    // Render-local AC: the served body is clean markdown (frontmatter is
    // stripped at parse time), so `co serve` / the SPA renders it without
    // provenance leaking into the rendered HTML.
    let body = entry["body"].as_str().expect("body present");
    assert!(body.contains("# Intro"), "markdown body is renderable");
    assert!(
        !body.contains("source:") && !body.contains("requested_by:"),
        "frontmatter must not leak into the renderable body"
    );
}

#[tokio::test]
async fn test_republish_unchanged_is_noop() {
    let dir = tempdir().unwrap();
    seed_public_universe(dir.path());
    let app = build_test_router(dir.path());
    let path = seed_imported_entry(&app, dir.path()).await;

    let payload = json!({
        "path": path,
        "requested_by": "CO-419",
        "commit_type": "docs",
        "commit_subject": "publish intro",
    });

    let (s1, first) = publish_req(&app, dir.path(), payload.clone()).await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(first["published"], true, "first publish records a commit");
    let sha1 = first["published_sha"].as_str().unwrap().to_string();

    // Re-publish identical content ⇒ idempotent no-op (same sha, published=false).
    let (s2, second) = publish_req(&app, dir.path(), payload).await;
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(
        second["published"], false,
        "re-publish unchanged ⇒ no new commit"
    );
    assert_eq!(
        second["published_sha"].as_str().unwrap(),
        sha1,
        "idempotency key stable"
    );
}

#[tokio::test]
async fn test_publish_requires_requested_by() {
    let dir = tempdir().unwrap();
    seed_public_universe(dir.path());
    let app = build_test_router(dir.path());
    let path = seed_imported_entry(&app, dir.path()).await;

    let (status, _) = publish_req(
        &app,
        dir.path(),
        json!({ "path": path, "requested_by": "" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_publish_owner_only() {
    let dir = tempdir().unwrap();
    seed_public_universe(dir.path());
    let app = build_test_router(dir.path());
    let path = seed_imported_entry(&app, dir.path()).await;

    let intruder = bearer_for(dir.path(), "intruder");
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{SLUG}/publish"))
                .header(header::AUTHORIZATION, intruder)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "path": path, "requested_by": "CO-419" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
