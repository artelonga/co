use super::super::*;
use super::support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tempfile::tempdir;
use tower::ServiceExt;

// -----------------------------------------------------------------------
// E2E: anon submits → hidden from public → owner approves → visible
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_suggest_review_approve_lifecycle() {
    let dir = tempdir().unwrap();
    seed_public_universe(dir.path());
    let app = build_test_router(dir.path());

    // 1. Anon (IP .10) submits a suggestion.
    let suggested = anon_suggest(&app, "10.0.0.10", "Anon proposal").await;
    assert_eq!(suggested["review_status"], "draft");
    let path = suggested["path"].as_str().unwrap().to_string();
    assert!(path.starts_with("suggestions/"));

    // 2. A DIFFERENT public reader (no cookie/IP) must NOT see the draft.
    let public_paths = list_entry_paths(&app, None).await;
    assert!(
        !public_paths.contains(&path),
        "draft must be hidden from the public listing"
    );

    // 3. Owner sees it in the review queue.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/universes/{SLUG}/review"))
                .header(header::AUTHORIZATION, owner_bearer(dir.path()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let queue = body_json(resp.into_body()).await;
    assert_eq!(queue["total"], 1, "one pending suggestion");
    assert_eq!(queue["entries"][0]["path"], path);

    // 4. Owner approves.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{SLUG}/review/approve"))
                .header(header::AUTHORIZATION, owner_bearer(dir.path()))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "path": path }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. Now the public reader sees it.
    let public_paths = list_entry_paths(&app, None).await;
    assert!(
        public_paths.contains(&path),
        "approved entry must appear in the public listing"
    );

    // 6. The review queue is now empty.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/universes/{SLUG}/review"))
                .header(header::AUTHORIZATION, owner_bearer(dir.path()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let queue = body_json(resp.into_body()).await;
    assert_eq!(queue["total"], 0, "queue empty after approval");
}

// -----------------------------------------------------------------------
// Submitter sees their own draft; reject removes it
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_submitter_sees_own_draft_and_reject_removes() {
    let dir = tempdir().unwrap();
    seed_public_universe(dir.path());
    let app = build_test_router(dir.path());

    let suggested = anon_suggest(&app, "10.0.0.20", "Mine").await;
    let path = suggested["path"].as_str().unwrap().to_string();

    // Submitter (same IP) DOES see their own draft.
    let mine = list_entry_paths(&app, Some("10.0.0.20")).await;
    assert!(mine.contains(&path), "submitter sees own draft");

    // Owner rejects → entry deleted.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{SLUG}/review/reject"))
                .header(header::AUTHORIZATION, owner_bearer(dir.path()))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "path": path }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Gone from the submitter's view too.
    let mine = list_entry_paths(&app, Some("10.0.0.20")).await;
    assert!(!mine.contains(&path), "rejected entry is deleted");
}

// -----------------------------------------------------------------------
// Non-owner cannot reach the review queue
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_review_queue_owner_only() {
    let dir = tempdir().unwrap();
    seed_public_universe(dir.path());
    let app = build_test_router(dir.path());

    let intruder = bearer_for(dir.path(), "intruder");
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/universes/{SLUG}/review"))
                .header(header::AUTHORIZATION, intruder)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// -----------------------------------------------------------------------
// Honeypot: a filled honeypot is accepted but writes nothing
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_honeypot_discards_submission() {
    let dir = tempdir().unwrap();
    seed_public_universe(dir.path());
    let app = build_test_router(dir.path());

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{SLUG}/suggest"))
                .header("x-forwarded-for", "10.0.0.30")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({ "title": "spam", "honeypot": "i-am-a-bot" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // Owner queue stays empty — nothing was written.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/universes/{SLUG}/review"))
                .header(header::AUTHORIZATION, owner_bearer(dir.path()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let queue = body_json(resp.into_body()).await;
    assert_eq!(queue["total"], 0, "honeypot submission must not be stored");
}

// -----------------------------------------------------------------------
// CO-418: conventional-commit + semver helpers (unit)
// -----------------------------------------------------------------------

#[test]
fn test_conventional_commit_message_with_scope() {
    let msg = conventional_commit_message("feat", Some("pipeline"), "publish docs/intro.md");
    assert_eq!(msg, "feat(pipeline): publish docs/intro.md");
}

#[test]
fn test_conventional_commit_message_no_scope() {
    let msg = conventional_commit_message("docs", None, "publish x");
    assert_eq!(msg, "docs: publish x");
}

#[test]
fn test_semver_bump_mapping() {
    assert_eq!(semver_bump_for("feat"), "minor");
    assert_eq!(semver_bump_for("fix"), "patch");
    assert_eq!(semver_bump_for("docs"), "patch");
    assert_eq!(semver_bump_for("chore"), "none");
}

#[test]
fn test_publish_sha_is_stable_and_content_sensitive() {
    let a = publish_sha("body", "github:o/r@sha", "CO-418", "docs: publish x");
    let b = publish_sha("body", "github:o/r@sha", "CO-418", "docs: publish x");
    assert_eq!(a, b, "same inputs ⇒ same hash (idempotency key)");
    let c = publish_sha(
        "body changed",
        "github:o/r@sha",
        "CO-418",
        "docs: publish x",
    );
    assert_ne!(a, c, "changed body ⇒ different hash");
}
