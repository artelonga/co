//! CO-438 (Bug 3a): a bulk import as an admin must not hit the global
//! write rate limit. The middleware capped admin writes at 60/min (CO-397)
//! *before* the vault handler's own admin-exemption ran, so `co source add` of
//! a repo with >60 files 429'd mid-import with no way to reach the tail. Admin
//! requests to the Vault API now bypass the global limiter (mirroring
//! `vault_auth`), so >60 sequential PUTs all succeed.

use super::support::*;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn admin_bulk_vault_writes_are_not_rate_limited() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();

    // create_user is admin-tier; own the target universe so the visibility gate
    // and write path admit the api-token holder.
    let admin = storage.create_user("admin@example.com", "Admin").unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "bulk-uni".into(),
                name: "Bulk".into(),
                description: String::new(),
            },
            &admin.id,
        )
        .unwrap();
    let token = storage
        .create_api_token(&admin.id, "import")
        .unwrap()
        .token
        .expect("raw token at creation");

    let (router, _tmp) = make_universe_router(storage, dir.path());

    // Well past the 60-writes/min admin cap the middleware used to enforce.
    let total = 75;
    for i in 0..total {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/universes/bulk-uni/vault/content/f{i}.md"))
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "text/markdown")
                    .body(Body::from(format!("# File {i}\n\nbody {i}\n")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "admin vault PUT #{i} was rate-limited; admin bulk-sync must be exempt"
        );
        assert!(
            resp.status().is_success(),
            "admin vault PUT #{i} failed with {}",
            resp.status()
        );
    }
}
