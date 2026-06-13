//! CO-444 — Federation API: token-auth universe creation + visibility + invites
//! + public-subscribe (CO side of YG-138).
//!
//! Every route here is exercised in-process (tower::ServiceExt, no port). The
//! central guarantee is that an **API token** works wherever a session JWT does
//! and resolves to the same owner (CO-161/CO-438 parity).

use super::support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn req(method: &str, uri: &str, token: &str, body: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json");
    if body.is_some() {
        b = b.header("Content-Type", "application/json");
    }
    b.body(
        body.map(|s| Body::from(s.to_string()))
            .unwrap_or(Body::empty()),
    )
    .unwrap()
}

async fn json(resp: axum::http::Response<Body>) -> serde_json::Value {
    let s = body_bytes(resp).await;
    serde_json::from_str(&s).unwrap_or(serde_json::Value::Null)
}

// --- Create (token-auth) -------------------------------------------------

/// POST /api/v1/universes with an **API token** creates a universe owned by the
/// token's user. This is the core regression CO-444 fixes (was 401 session-only).
#[tokio::test]
async fn create_universe_by_api_token_is_201_and_owned_by_token_user() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("svc@example.com", "Service").unwrap();
    let token = storage
        .create_api_token(&owner.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes",
            &token,
            Some(r#"{"key":"fed-uni","name":"Fed","visibility":"public"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "token create must 201");
    let body = json(resp).await;
    assert_eq!(body["key"], "fed-uni");
    assert_eq!(body["owner_id"], owner.id);
    // `public` is stored canonically as `public-subscribable`.
    assert_eq!(body["visibility"], "public-subscribable");
}

/// Same owner whether created via token or via session JWT.
#[tokio::test]
async fn create_universe_owner_matches_for_token_and_session() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let user = storage.create_user("dual@example.com", "Dual").unwrap();
    let token = storage
        .create_api_token(&user.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (jwt, _) =
        crate::auth::sign_jwt(&user.id, "dual@example.com", "admin", "test-jwt-secret").unwrap();
    let (router, _t) = make_universe_router(storage, dir.path());

    let by_token = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes",
            &token,
            Some(r#"{"key":"via-token","name":"T"}"#),
        ))
        .await
        .unwrap();
    let by_session = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes",
            &jwt,
            Some(r#"{"key":"via-session","name":"S"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(by_token.status(), StatusCode::CREATED);
    assert_eq!(by_session.status(), StatusCode::CREATED);
    assert_eq!(json(by_token).await["owner_id"], user.id);
    assert_eq!(json(by_session).await["owner_id"], user.id);
}

/// `unlisted` is accepted and stored verbatim; `parent_key` re-parents at create.
#[tokio::test]
async fn create_universe_with_unlisted_and_parent_key() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("p@example.com", "P").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "parent-uni".into(),
                name: "Parent".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    let token = storage
        .create_api_token(&owner.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes",
            &token,
            Some(r#"{"key":"child-uni","name":"Child","visibility":"unlisted","parent_key":"parent-uni"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = json(resp).await;
    assert_eq!(body["visibility"], "unlisted");
    assert_eq!(body["parent_key"], "parent-uni");
}

/// Invalid visibility → 400, no universe created.
#[tokio::test]
async fn create_universe_invalid_visibility_is_400() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("bad@example.com", "Bad").unwrap();
    let token = storage
        .create_api_token(&owner.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes",
            &token,
            Some(r#"{"key":"nope-uni","name":"Nope","visibility":"secret"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    // No orphan row left behind.
    let resp2 = router
        .clone()
        .oneshot(req("GET", "/api/v1/universes/nope-uni", &token, None))
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::NOT_FOUND);
}

/// Unknown parent_key → 400, no orphan universe (CO-438 no-orphan rule).
#[tokio::test]
async fn create_universe_unknown_parent_is_400_no_orphan() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("o@example.com", "O").unwrap();
    let token = storage
        .create_api_token(&owner.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes",
            &token,
            Some(r#"{"key":"orphan-uni","name":"X","parent_key":"ghost"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let resp2 = router
        .clone()
        .oneshot(req("GET", "/api/v1/universes/orphan-uni", &token, None))
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::NOT_FOUND);
}

/// Duplicate key → 409 (idempotent guard, no second row).
#[tokio::test]
async fn create_universe_duplicate_key_is_409() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("dup@example.com", "Dup").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "taken".into(),
                name: "Taken".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    let token = storage
        .create_api_token(&owner.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes",
            &token,
            Some(r#"{"key":"taken","name":"Again"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

// --- Visibility (PUT) ----------------------------------------------------

/// PUT visibility=public via API token flips the universe to public-subscribable.
#[tokio::test]
async fn update_visibility_via_token() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("v@example.com", "V").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "vis-uni".into(),
                name: "Vis".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    let token = storage
        .create_api_token(&owner.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "PUT",
            "/api/v1/universes/vis-uni",
            &token,
            Some(r#"{"visibility":"public"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(json(resp).await["visibility"], "public-subscribable");
}

// --- Public subscribe ----------------------------------------------------

/// Subscribe to a public-subscribable universe via API token → 204.
#[tokio::test]
async fn subscribe_public_universe_via_token() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("owner-s@example.com", "Owner").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "pub-uni".into(),
                name: "Pub".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    set_visibility(&storage, "pub-uni", "public-subscribable");
    let subscriber = storage.create_user("sub@example.com", "Sub").unwrap();
    let token = storage
        .create_api_token(&subscriber.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes/pub-uni/subscribe",
            &token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = router
        .clone()
        .oneshot(req(
            "DELETE",
            "/api/v1/universes/pub-uni/subscribe",
            &token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// --- Invites (/invites) --------------------------------------------------

/// Create → list → revoke an invite via the `/invites` surface (API token).
#[tokio::test]
async fn invites_create_list_revoke_via_token() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("owner-i@example.com", "Owner").unwrap();
    let _guest = storage.create_user("guest-i@example.com", "Guest").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "inv-uni".into(),
                name: "Inv".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    let token = storage
        .create_api_token(&owner.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    // Create (note: `role`, and email recipient).
    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes/inv-uni/invites",
            &token,
            Some(r#"{"email":"guest-i@example.com","role":"member"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let token_hash = json(resp).await["token_hash"].as_str().unwrap().to_string();

    // List shows one pending invite.
    let resp = router
        .clone()
        .oneshot(req(
            "GET",
            "/api/v1/universes/inv-uni/invites",
            &token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let items = json(resp).await;
    assert_eq!(items.as_array().unwrap().len(), 1);

    // Revoke by token_hash → 204.
    let resp = router
        .clone()
        .oneshot(req(
            "DELETE",
            &format!("/api/v1/universes/inv-uni/invites/{token_hash}"),
            &token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // List is now empty (revoked filtered out).
    let resp = router
        .clone()
        .oneshot(req(
            "GET",
            "/api/v1/universes/inv-uni/invites",
            &token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(json(resp).await.as_array().unwrap().len(), 0);
}

/// A non-owner/non-admin cannot create invites → 403.
#[tokio::test]
async fn invites_create_forbidden_for_non_owner() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("owner-f@example.com", "Owner").unwrap();
    let other = storage.create_user("other-f@example.com", "Other").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "inv-f".into(),
                name: "Inv".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    let token = storage
        .create_api_token(&other.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes/inv-f/invites",
            &token,
            Some(r#"{"email":"x@example.com"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// Accept an invite via the universe-scoped path; the invitee becomes a member.
#[tokio::test]
async fn invites_accept_universe_scoped() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("owner-a@example.com", "Owner").unwrap();
    let guest = storage.create_user("guest-a@example.com", "Guest").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "acc-uni".into(),
                name: "Acc".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    // Mint an invite directly to capture the raw token (the API returns only the hash).
    let raw_token = storage
        .create_invitation("acc-uni", &owner.id, None, Some(&guest.id), "member")
        .unwrap()
        .1;
    let guest_token = storage
        .create_api_token(&guest.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            &format!("/api/v1/universes/acc-uni/invites/{raw_token}/accept"),
            &guest_token,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "accept should succeed for the invited user"
    );
    let body = json(resp).await;
    assert_eq!(body["universe_key"], "acc-uni");
    assert_eq!(body["role"], "member");
}

/// `handle` is accepted as an alias for `usuario` when inviting.
#[tokio::test]
async fn invites_create_by_handle_alias() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();
    let owner = storage.create_user("owner-h@example.com", "Owner").unwrap();
    let guest = storage.create_user("guest-h@example.com", "Guest").unwrap();
    // Give the guest a usuario/handle to be found by.
    storage
        .conn()
        .execute(
            "UPDATE users SET usuario = 'guesth' WHERE id = ?1",
            rusqlite::params![guest.id],
        )
        .unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "inv-h".into(),
                name: "Inv".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    let token = storage
        .create_api_token(&owner.id, "yg")
        .unwrap()
        .token
        .expect("raw token");
    let (router, _t) = make_universe_router(storage, dir.path());

    let resp = router
        .clone()
        .oneshot(req(
            "POST",
            "/api/v1/universes/inv-h/invites",
            &token,
            Some(r#"{"handle":"guesth","role":"member"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "handle alias should resolve the invitee"
    );
}
