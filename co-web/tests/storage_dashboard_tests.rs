//! Integration tests for `GET /api/v1/admin/storage` (2.7.27).
//!
//! The endpoint summarises per-universe disk + table footprints so
//! the owner can spot storage hotspots before they bite. Admin-only.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
use co_web::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use co_web::storage::{Storage, seed_data};

extern crate co;

fn test_config(dir: &std::path::Path) -> WebConfig {
    WebConfig {
        port: 3000,
        data_dir: dir.to_str().unwrap().to_string(),
        static_dir: "co-web/static".to_string(),
        default_variant: "a".to_string(),
        experiments: true,
        plugins_dir: "plugins".to_string(),
        game_db_path: None,
        universo_dir: "quilomboaraucaria".to_string(),
        gestao_github_admins: vec!["artelonga".to_string()],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: true,
    }
}

fn admin_bearer() -> String {
    // CO admin email allowlist key is yuri@artelonga.com.br; sign a
    // JWT for that subject.
    let (token, _) = co_web::auth::sign_jwt(
        "admin-user",
        "yuri@artelonga.com.br",
        "admin",
        "dev-secret-change-me",
    )
    .unwrap();
    format!("Bearer {token}")
}

fn non_admin_bearer() -> String {
    let (token, _) = co_web::auth::sign_jwt(
        "regular-user",
        "regular@example.com",
        "player",
        "dev-secret-change-me",
    )
    .unwrap();
    format!("Bearer {token}")
}

fn build_test_router(dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);
    seed_data(&mut storage);
    // Seed template so there's at least one universe row.
    storage.seed_template_universe();
    let experiment = ExperimentStore::new(&config.data_dir);

    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("game storage"),
    );
    let state: AppState = AppState::new(AppStateInner {
        core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
        realtime: Arc::new(RealtimeState {
            doc_rooms: co_web::ws::new_room_manager(),
            sync_rooms: co_web::sync_ws::new_sync_room_manager(),
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        }),
        index: Arc::new(IndexState {
            cache: co_web::cache::CacheLayer::new(),
            embeddings: std::sync::Arc::new(co_web::embedding::EmbeddingService::disabled()),
            embedding_tx: {
                let (tx, _) = co_web::embedding_worker::channel();
                tx
            },
        }),
        integrations: Arc::new(IntegrationsState {
            mail,
            geo: std::sync::Arc::new(co_web::geo::GeoDb::disabled()),
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            game_storage,
            wae: co_web::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(co_web::auth::JwtKey::load_or_generate()),
            rate_limiter: std::sync::Mutex::new(co_web::rate_limit::RateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: co_web::worker_supervisor::WorkerSupervisor::new(),
        }),
    });
    build_router(state, None)
}

async fn body_to_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn storage_dashboard_requires_admin() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // No auth -> 401.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/storage")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Non-admin auth -> 403.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/storage")
                .header(header::AUTHORIZATION, non_admin_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn me_storage_returns_owned_universes_only() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // Seed two universes: one owned by test-user, one by someone else.
    let storage = Storage::new(dir.path());
    // non_admin_bearer signs JWT with sub = "regular-user", so that's
    // the owner_id resolve_user_id will produce.
    for (key, owner) in [("mine", "regular-user"), ("yours", "other-user")] {
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
                 VALUES (?1, ?2, ?3, '2026-01-01')",
                rusqlite::params![owner, format!("{owner}@t.l"), owner],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, is_public, visibility) \
                 VALUES (?1, ?1, '', ?2, '2026-01-01', 1, 'public-subscribable')",
                rusqlite::params![key, owner],
            )
            .unwrap();
    }

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/me/storage")
                .header(header::AUTHORIZATION, non_admin_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_to_json(res.into_body()).await;
    let keys: Vec<&str> = body["universes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&"mine"), "should include owned 'mine'");
    assert!(
        !keys.contains(&"yours"),
        "should NOT include other user's 'yours'"
    );
    assert!(
        !keys.contains(&"template"),
        "should NOT include template (owned by 'system')"
    );
}

#[tokio::test]
async fn universe_storage_owner_only() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());
    let storage = Storage::new(dir.path());
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
             VALUES (?1, ?2, ?3, '2026-01-01')",
            rusqlite::params!["regular-user", "t@t", "T"],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, owner_id, created_at, is_public, visibility) \
             VALUES ('owned-by-me', 'mine', 'regular-user', '2026-01-01', 1, 'public-subscribable')",
            [],
        )
        .unwrap();
    // Private universe so the visibility check exercises the
    // membership branch (not the is_public=1 short-circuit).
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
             VALUES (?1, ?2, ?3, '2026-01-01')",
            rusqlite::params!["other-user", "o@o", "O"],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, owner_id, created_at, is_public, visibility) \
             VALUES ('owned-by-other', 'theirs', 'other-user', '2026-01-01', 0, 'private')",
            [],
        )
        .unwrap();

    // Owner can drill in.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/owned-by-me/storage")
                .header(header::AUTHORIZATION, non_admin_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_to_json(res.into_body()).await;
    assert_eq!(body["universes"][0]["key"], "owned-by-me");

    // Non-owner is denied.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/owned-by-other/storage")
                .header(header::AUTHORIZATION, non_admin_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn storage_dashboard_returns_shape() {
    unsafe {
        std::env::set_var("JWT_SECRET", "dev-secret-change-me");
        std::env::set_var("CO_SEED_ADMIN_EMAIL", "yuri@artelonga.com.br");
    }
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/admin/storage")
                .header(header::AUTHORIZATION, admin_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = body_to_json(res.into_body()).await;
    assert_eq!(status, StatusCode::OK, "expected 200; got {status}: {body}");

    assert!(body["generated_at"].is_string());
    assert!(body["host"]["data_dir"].is_string());
    assert!(body["totals"]["universes"].is_number());
    let universes = body["universes"].as_array().expect("universes array");
    assert!(!universes.is_empty(), "should have at least template");

    let t = universes
        .iter()
        .find(|u| u["key"] == "template")
        .expect("template universe in dashboard");
    assert!(t["data_db_bytes"].as_u64().is_some());
    assert!(t["md_bytes"].as_u64().is_some());
    assert!(t["tables"]["entries"]["rows"].is_number());
    assert!(t["tables"]["entry_events"]["rows"].is_number());
}

#[tokio::test]
async fn data_db_bytes_nonzero_after_vault_put() {
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-change-me") };
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // Seed a universe owned by "regular-user" using direct storage access
    // (same pattern as other tests in this file).
    let storage = Storage::new(dir.path());
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
             VALUES (?1, ?2, ?3, '2026-01-01')",
            rusqlite::params!["regular-user", "regular@example.com", "Regular User"],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, owner_id, created_at, is_public, visibility) \
             VALUES ('co240-fix', 'CO-240 Fix', 'regular-user', '2026-01-01', 1, 'public-subscribable')",
            [],
        )
        .unwrap();
    drop(storage);

    let bearer = non_admin_bearer();

    // PUT 5 entries via vault API — materialises the per-universe data.db.
    for i in 1..=5u32 {
        let content = format!("---\ntitle: Entry {i}\ntype: note\n---\n\nBody {i}");
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/api/v1/universes/co240-fix/vault/notes/entry{i}.md"
                    ))
                    .header(header::AUTHORIZATION, &bearer)
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            res.status().is_success(),
            "vault PUT entry {i} failed: {}",
            res.status()
        );
    }

    // Query the per-universe storage dashboard.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co240-fix/storage")
                .header(header::AUTHORIZATION, &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_to_json(res.into_body()).await;

    let universe = &body["universes"][0];
    assert_eq!(universe["key"], "co240-fix");
    assert!(
        universe["data_db_bytes"].as_u64().unwrap_or(0) > 0,
        "data_db_bytes should be > 0 after 5 vault PUTs, got: {}",
        universe["data_db_bytes"]
    );
}
