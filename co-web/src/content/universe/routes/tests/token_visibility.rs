//! CO-438 (Bug 2): `GET /api/v1/universes/{key}` for a *private* universe must
//! treat an API token and a session JWT identically. The owner reads their own
//! private universe with either credential (200); a non-owner gets 404 with
//! either. Before the fix, `get_universe_info` decoded JWTs only, so an
//! API-token request — even by the owner — resolved to anonymous and 404'd,
//! which broke `co source add`'s `universe_exists` probe (it uses the token).

use super::support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// GET a private universe by its owner's **API token** → 200 (the regression).
#[tokio::test]
async fn private_universe_get_by_owner_api_token_is_200() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();

    let owner = storage.create_user("owner@example.com", "Owner").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "secret-uni".into(),
                name: "Secret".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    // create_universe defaults to visibility=private; assert the precondition.
    assert_eq!(
        storage.get_universe("secret-uni").unwrap().visibility,
        "private"
    );
    let owner_token = storage
        .create_api_token(&owner.id, "import")
        .unwrap()
        .token
        .expect("raw token at creation");

    let (router, _tmp) = make_universe_router(storage, dir.path());

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/secret-uni")
                .header("Authorization", format!("Bearer {owner_token}"))
                .header("Accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "owner must read their own private universe by api-token"
    );
    let body = body_bytes(resp).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["key"], "secret-uni");
}

/// GET a private universe by its owner's **session JWT** → 200 (parity check).
#[tokio::test]
async fn private_universe_get_by_owner_session_is_200() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();

    let owner = storage.create_user("owner2@example.com", "Owner2").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "secret-uni-2".into(),
                name: "Secret2".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();

    let (router, _tmp) = make_universe_router(storage, dir.path());

    let (jwt, _) =
        crate::auth::sign_jwt(&owner.id, "owner2@example.com", "admin", "test-jwt-secret").unwrap();

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/secret-uni-2")
                .header("Authorization", format!("Bearer {jwt}"))
                .header("Accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

/// GET a private universe by a **non-owner's API token** → 404. The fix must
/// not leak private universes to other token holders.
#[tokio::test]
async fn private_universe_get_by_non_owner_api_token_is_404() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();

    let owner = storage.create_user("owner3@example.com", "Owner3").unwrap();
    let stranger = storage
        .create_user("stranger@example.com", "Stranger")
        .unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "secret-uni-3".into(),
                name: "Secret3".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    let stranger_token = storage
        .create_api_token(&stranger.id, "snoop")
        .unwrap()
        .token
        .expect("raw token at creation");

    let (router, _tmp) = make_universe_router(storage, dir.path());

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/secret-uni-3")
                .header("Authorization", format!("Bearer {stranger_token}"))
                .header("Accept", "application/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "a non-owner token must not see another user's private universe"
    );
}
