use super::support::*;
use rusqlite::params;

#[tokio::test]
async fn test_reparent_universe_owner_can_set_and_clear() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES ('usr_rp','rp@example.com','RP','player',?1,'rp')",
            params![now],
        )
        .unwrap();

    let (router, _tmp) = make_universe_router(storage, dir.path());
    let (token, _) =
        crate::auth::sign_jwt("usr_rp", "rp@example.com", "player", "test-jwt-secret").unwrap();

    // Create parent + child universes via the API (so the caller owns both).
    assert_eq!(
        create_universe_via_api(&router, &token, "yuri", "Yuri").await,
        axum::http::StatusCode::CREATED
    );
    assert_eq!(
        create_universe_via_api(&router, &token, "nlp", "NLP").await,
        axum::http::StatusCode::CREATED
    );

    // Re-parent nlp under yuri.
    let put_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/nlp")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"parent_key":"yuri"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_resp.status(), axum::http::StatusCode::OK);
    let body = body_bytes(put_resp).await;
    let resp: super::super::UpdateUniverseResponse = serde_json::from_str(&body).unwrap();
    assert_eq!(resp.parent_key.as_deref(), Some("yuri"));

    // Clear the parent with an empty string.
    let clear_resp = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/nlp")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"parent_key":""}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(clear_resp.status(), axum::http::StatusCode::OK);
    let body = body_bytes(clear_resp).await;
    let resp: super::super::UpdateUniverseResponse = serde_json::from_str(&body).unwrap();
    assert!(resp.parent_key.is_none());
}

/// CO-423: a non-owner cannot re-parent a universe (403).
#[tokio::test]
async fn test_reparent_universe_non_owner_forbidden() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES ('usr_owner','owner@example.com','Owner','player',?1,'owner'), \
                    ('usr_other','other@example.com','Other','player',?1,'other')",
            params![now],
        )
        .unwrap();

    let (router, _tmp) = make_universe_router(storage, dir.path());
    let (owner_token, _) = crate::auth::sign_jwt(
        "usr_owner",
        "owner@example.com",
        "player",
        "test-jwt-secret",
    )
    .unwrap();
    let (other_token, _) = crate::auth::sign_jwt(
        "usr_other",
        "other@example.com",
        "player",
        "test-jwt-secret",
    )
    .unwrap();

    assert_eq!(
        create_universe_via_api(&router, &owner_token, "yuri", "Yuri").await,
        axum::http::StatusCode::CREATED
    );
    assert_eq!(
        create_universe_via_api(&router, &owner_token, "child", "Child").await,
        axum::http::StatusCode::CREATED
    );

    let put_resp = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/child")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {other_token}"))
                .body(Body::from(r#"{"parent_key":"yuri"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_resp.status(), axum::http::StatusCode::FORBIDDEN);
}

/// CO-423: re-parenting to a nonexistent parent is rejected (400).
#[tokio::test]
async fn test_reparent_universe_nonexistent_parent_rejected() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES ('usr_np','np@example.com','NP','player',?1,'np')",
            params![now],
        )
        .unwrap();

    let (router, _tmp) = make_universe_router(storage, dir.path());
    let (token, _) =
        crate::auth::sign_jwt("usr_np", "np@example.com", "player", "test-jwt-secret").unwrap();

    assert_eq!(
        create_universe_via_api(&router, &token, "solo", "Solo").await,
        axum::http::StatusCode::CREATED
    );

    let put_resp = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/solo")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(r#"{"parent_key":"does-not-exist"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_resp.status(), axum::http::StatusCode::BAD_REQUEST);
}

/// DELETE /api/v1/universes/:slug returns typed DeleteUniverseResponse.
#[tokio::test]
async fn test_delete_universe_returns_typed_response() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();

    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES ('usr_del','del@example.com','Del','player',?1,'del')",
            params![now],
        )
        .unwrap();

    let (router, _tmp) = make_universe_router(storage, dir.path());
    let (token, _) =
        crate::auth::sign_jwt("usr_del", "del@example.com", "player", "test-jwt-secret").unwrap();

    // Create a universe to delete
    let create_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::from(
                    r#"{"key":"to-delete","name":"To Delete","description":""}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), axum::http::StatusCode::CREATED);

    let del_resp = router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/to-delete")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(del_resp.status(), axum::http::StatusCode::OK);
    let body = body_bytes(del_resp).await;
    let resp: super::super::DeleteUniverseResponse = serde_json::from_str(&body).unwrap();
    assert_eq!(resp.deleted, "to-delete");
}
