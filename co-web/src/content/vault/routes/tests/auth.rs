use super::support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tempfile::tempdir;
use tower::ServiceExt;

// -----------------------------------------------------------------------
// Auth — unauthenticated request rejected
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_vault_requires_auth() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "auth-test");
    let app = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/auth-test/vault/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// -----------------------------------------------------------------------
// API token lifecycle
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_api_token_lifecycle() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());
    let bearer = test_bearer();

    // Create token
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/token")
                .header(header::AUTHORIZATION, &bearer)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"obsidian-plugin"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created: serde_json::Value =
        serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    let token_val = created["token"].as_str().unwrap().to_string();
    let token_id = created["id"].as_str().unwrap().to_string();
    assert!(
        token_val.starts_with("co_"),
        "Token should have co_ prefix: {token_val}"
    );

    // List tokens
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/tokens")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let list: serde_json::Value = serde_json::from_str(&body_str(resp.into_body()).await).unwrap();
    assert!(list.as_array().unwrap().iter().any(|t| t["id"] == token_id));

    // Use the API token to access vault
    seed_universe(dir.path(), "token-test");
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/token-test/vault/")
                .header(header::AUTHORIZATION, format!("Bearer {token_val}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Revoke token
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/auth/tokens/{token_id}"))
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Revoked token should no longer work
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/token-test/vault/")
                .header(header::AUTHORIZATION, format!("Bearer {token_val}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// -----------------------------------------------------------------------
// CO-237: API token hashing at rest
// -----------------------------------------------------------------------

#[test]
fn test_api_token_hash_at_rest() {
    let dir = tempdir().unwrap();
    let storage = crate::storage::Storage::new(dir.path().to_str().unwrap());

    let tok = storage
        .create_api_token("user-hash-test", "hash-test-token")
        .unwrap();

    // Raw token returned to caller
    let raw = tok
        .token
        .clone()
        .expect("raw token must be Some on creation");
    assert!(raw.starts_with("co_"), "token must have co_ prefix");

    // token_hash and token_prefix must be populated
    assert!(!tok.token_hash.is_empty(), "token_hash must not be empty");
    assert!(
        tok.token_prefix.starts_with("co_"),
        "token_prefix must start with co_"
    );

    // DB must NOT contain the raw co_<value> token — only the hash
    let conn = rusqlite::Connection::open(dir.path().join("meta.db")).unwrap();
    let stored_token: String = conn
        .query_row(
            "SELECT token FROM api_tokens WHERE id = ?1",
            rusqlite::params![tok.id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !stored_token.starts_with("co_"),
        "plaintext token must not be in DB; got: {stored_token:?}"
    );

    let stored_hash: String = conn
        .query_row(
            "SELECT token_hash FROM api_tokens WHERE id = ?1",
            rusqlite::params![tok.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored_hash, tok.token_hash,
        "stored hash must match returned hash"
    );

    // Lookup by raw value must succeed and return the same token id
    let found = storage
        .get_api_token_by_value(&raw)
        .unwrap()
        .expect("token must be found by raw value");
    assert_eq!(found.id, tok.id);
    assert!(
        found.token.is_none(),
        "raw token must not be returned on lookup"
    );

    // Lookup with wrong value must return None
    let not_found = storage
        .get_api_token_by_value("co_thisiswrong1234567890")
        .unwrap();
    assert!(not_found.is_none(), "wrong token must not match");
}
