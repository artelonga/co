use super::support::*;
use rusqlite::params;

// --- CO-66: 409 on duplicate universe key ---

/// POST /api/v1/universes with an existing key returns 409 Conflict.
#[tokio::test]
async fn test_create_universe_duplicate_key_returns_409() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();

    // Pre-create the universe directly in storage.
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "dupe-uni".into(),
                name: "Dupe Universe".into(),
                description: String::new(),
            },
            "usr_owner",
        )
        .unwrap();

    let (router, _tmp) = make_universe_router(storage, dir.path());

    let (token, _) = crate::auth::sign_jwt(
        "usr_owner",
        "owner@example.com",
        "player",
        "test-jwt-secret",
    )
    .unwrap();

    let payload = serde_json::json!({
        "key": "dupe-uni",
        "name": "Another Universe",
        "description": ""
    });
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::CONFLICT);
    let body = body_bytes(response).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["error"], "conflict");
}

/// PUT /api/v1/universes/:slug returns typed UpdateUniverseResponse with all updated fields.
#[tokio::test]
async fn test_update_universe_request_and_response_typed() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();

    // Insert a test user and universe for the owner
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES ('usr_update','update@example.com','Update','player',?1,'update')",
            params![now],
        )
        .unwrap();

    let (router, _tmp) = make_universe_router(storage, dir.path());
    let (token, _) = crate::auth::sign_jwt(
        "usr_update",
        "update@example.com",
        "player",
        "test-jwt-secret",
    )
    .unwrap();

    let payload = serde_json::json!({
        "key": "upd-uni",
        "name": "Before Update",
        "description": ""
    });
    let create_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), axum::http::StatusCode::CREATED);

    let put_body = r#"{"name":"After Update","description":"new desc"}"#;
    let put_resp = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/upd-uni")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(put_body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(put_resp.status(), axum::http::StatusCode::OK);
    let body = body_bytes(put_resp).await;
    let resp: super::super::UpdateUniverseResponse = serde_json::from_str(&body).unwrap();
    assert_eq!(resp.key, "upd-uni");
    assert_eq!(resp.name, "After Update");
    assert_eq!(resp.description, "new desc");
}

/// UpdateUniverseRequest parses correctly from JSON.
#[test]
fn test_update_universe_request_parses() {
    let json = r#"{"name":"New Name","visibility":"public-subscribable"}"#;
    let req: super::super::UpdateUniverseRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.name.as_deref(), Some("New Name"));
    assert_eq!(req.visibility.as_deref(), Some("public-subscribable"));
    assert!(req.description.is_none());
    assert!(req.parent_key.is_none());
}

/// CO-423: UpdateUniverseRequest parses `parent_key` (key, empty, absent).
#[test]
fn test_update_universe_request_parses_parent_key() {
    let with = r#"{"parent_key":"yuri"}"#;
    let req: super::super::UpdateUniverseRequest = serde_json::from_str(with).unwrap();
    assert_eq!(req.parent_key.as_deref(), Some("yuri"));

    let clear = r#"{"parent_key":""}"#;
    let req: super::super::UpdateUniverseRequest = serde_json::from_str(clear).unwrap();
    assert_eq!(req.parent_key.as_deref(), Some(""));

    let absent = r#"{"name":"x"}"#;
    let req: super::super::UpdateUniverseRequest = serde_json::from_str(absent).unwrap();
    assert!(req.parent_key.is_none());
}
