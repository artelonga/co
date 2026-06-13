//! CO-96: universe soft-delete + archive lifecycle route tests.

use super::support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::params;
use tower::ServiceExt;

/// Seed a user row and return a signed JWT for them.
fn seed_user_token(storage: &crate::storage::Storage, id: &str) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES (?1, ?2, 'Owner', 'player', ?3, ?1)",
            params![id, format!("{id}@example.com"), now],
        )
        .unwrap();
    crate::auth::sign_jwt(
        id,
        &format!("{id}@example.com"),
        "player",
        "test-jwt-secret",
    )
    .unwrap()
    .0
}

async fn list_keys(router: &axum::Router, token: &str) -> Vec<String> {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_bytes(resp).await;
    let arr: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
    arr.into_iter()
        .map(|u| u["key"].as_str().unwrap().to_string())
        .collect()
}

async fn post(router: &axum::Router, token: &str, uri: &str) -> StatusCode {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// DELETE soft-deletes: the universe vanishes from the listing but reappears in
/// the trash view, and `restore` brings it fully back.
#[tokio::test]
async fn test_delete_soft_deletes_and_restore() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let token = seed_user_token(&storage, "usr_del");
    let (router, _tmp) = make_universe_router(storage, dir.path());

    assert_eq!(
        create_universe_via_api(&router, &token, "doomed", "Doomed").await,
        StatusCode::CREATED
    );
    assert!(
        list_keys(&router, &token)
            .await
            .contains(&"doomed".to_string())
    );

    // Soft-delete.
    let del = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/doomed")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::OK);

    // Gone from the main listing…
    assert!(
        !list_keys(&router, &token)
            .await
            .contains(&"doomed".to_string())
    );

    // …but present in the trash view, flagged as deleted.
    let trash = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/trash")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(trash.status(), StatusCode::OK);
    let items: Vec<super::super::TrashedUniverse> =
        serde_json::from_str(&body_bytes(trash).await).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].key, "doomed");
    assert_eq!(items[0].state, "deleted");
    assert!(items[0].deleted_at.is_some());

    // Restore brings it back to the listing.
    assert_eq!(
        post(&router, &token, "/api/v1/universes/doomed/restore").await,
        StatusCode::OK
    );
    assert!(
        list_keys(&router, &token)
            .await
            .contains(&"doomed".to_string())
    );
}

/// Archive soft-hides a universe; restore un-hides it.
#[tokio::test]
async fn test_archive_then_restore() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let token = seed_user_token(&storage, "usr_arc");
    let (router, _tmp) = make_universe_router(storage, dir.path());

    assert_eq!(
        create_universe_via_api(&router, &token, "stale", "Stale").await,
        StatusCode::CREATED
    );

    assert_eq!(
        post(&router, &token, "/api/v1/universes/stale/archive").await,
        StatusCode::OK
    );
    assert!(
        !list_keys(&router, &token)
            .await
            .contains(&"stale".to_string())
    );

    let trash = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/trash")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let items: Vec<super::super::TrashedUniverse> =
        serde_json::from_str(&body_bytes(trash).await).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].state, "archived");

    assert_eq!(
        post(&router, &token, "/api/v1/universes/stale/restore").await,
        StatusCode::OK
    );
    assert!(
        list_keys(&router, &token)
            .await
            .contains(&"stale".to_string())
    );
}

/// The template universe is protected from deletion and archival.
#[tokio::test]
async fn test_cannot_delete_or_archive_template() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let token = seed_user_token(&storage, "usr_tmpl");
    let (router, _tmp) = make_universe_router(storage, dir.path());

    let del = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/universes/template")
                .header("Authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::BAD_REQUEST);

    assert_eq!(
        post(&router, &token, "/api/v1/universes/template/archive").await,
        StatusCode::BAD_REQUEST
    );
}
