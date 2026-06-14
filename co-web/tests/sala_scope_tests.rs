//! CO-399: integration tests for the multi-universe sala scope.
//!
//! The acceptance E2E: a two-universe subset sala must render nodes from BOTH
//! universes, while an anonymous viewer sees only the public slice. These tests
//! exercise the real router in-process (no port) over the CO-399 surfaces:
//!   · `GET  /api/v1/sala/scope?u=a,b`        — visibility-gated scope resolver
//!   · `GET  /api/v1/universes/{k}/entries`   — the per-universe picker source
//!   · `PUT/GET /api/v1/sala/state?scope=a,b` — scope-keyed state round-trip
//!   · `POST /api/v1/sala/state/share`        — multi-scope share token
//!   · `GET  /api/v1/workspace-states/{tok}`  — share-token read (carries scope)

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::auth::sign_jwt;
use co_web::config::WebConfig;
use co_web::domain::EntryDomain;
use co_web::experiment::ExperimentStore;
use co_web::models::CreateUniverse;
use co_web::server::{
    AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState, build_router,
};
use co_web::storage::Storage;

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
        gestao_github_admins: vec![],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: false,
    }
}

fn bearer(user_id: &str) -> String {
    let (token, _) = sign_jwt(
        user_id,
        &format!("{user_id}@test.local"),
        "player",
        "dev-secret-change-me",
    )
    .unwrap();
    format!("Bearer {token}")
}

fn seed_entry(storage: &Storage, universe_key: &str, path: &str, title: &str) {
    let store = storage.entry_store(universe_key).expect("entry store");
    store
        .upsert(&EntryDomain {
            path: path.to_string(),
            universe_key: universe_key.to_string(),
            entry_type: "nota".to_string(),
            title: Some(title.to_string()),
            frontmatter: serde_json::json!({ "title": title }),
            body: format!("# {title}"),
            body_hash: String::new(),
            created_at: None,
            updated_at: None,
        })
        .expect("seed entry");
}

/// Build a router pre-seeded with two universes owned by `owner-a`:
///   "pub-uni"  — public-subscribable (anyone can read), 1 entry
///   "priv-uni" — private (owner-only), 1 entry
/// Returns the router and the owner's user_id.
fn build_test_router(dir: &std::path::Path) -> (axum::Router, String) {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);

    let owner = storage.create_user("owner@test.local", "Owner").unwrap();

    storage
        .create_universe(
            CreateUniverse {
                key: "priv-uni".into(),
                name: "Private".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    storage
        .create_universe(
            CreateUniverse {
                key: "pub-uni".into(),
                name: "Public".into(),
                description: String::new(),
            },
            &owner.id,
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "UPDATE universes SET is_public = 1, visibility = 'public-subscribable' \
             WHERE key = 'pub-uni'",
            [],
        )
        .unwrap();

    seed_entry(&storage, "pub-uni", "notas/hello.md", "Hello Public");
    seed_entry(&storage, "priv-uni", "notas/secret.md", "Secret Private");

    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("Failed to open test game storage"),
    );
    let state: AppState = AppState::new(AppStateInner {
        core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
        realtime: Arc::new(RealtimeState {
            doc_rooms: co_web::ws::new_room_manager(),
            sync_rooms: co_web::sync_ws::new_sync_room_manager(),
            chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
            chat_presence: Mutex::new(std::collections::HashMap::new()),
        }),
        index: Arc::new(IndexState {
            cache: co_web::cache::CacheLayer::new(),
            embeddings: Arc::new(co_web::embedding::EmbeddingService::disabled()),
            embedding_tx: {
                let (tx, _) = co_web::embedding_worker::channel();
                tx
            },
        }),
        integrations: Arc::new(IntegrationsState {
            mail,
            geo: Arc::new(co_web::geo::GeoDb::disabled()),
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            game_storage,
            wae: co_web::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(co_web::auth::JwtKey::load_or_generate()),
            rate_limiter: Mutex::new(co_web::rate_limit::InProcessRateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: co_web::infra::workers::InProcessExecutor::new_arc(),
        }),
    });

    (build_router(state, None), owner.id)
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn scope_keys(v: &serde_json::Value) -> Vec<String> {
    v["universes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|u| u["key"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Owner resolving the two-universe subset sees BOTH universes — the picker will
/// fetch entries from each, so the canvas can render nodes from both.
#[tokio::test]
async fn owner_subset_scope_resolves_both_universes() {
    let dir = tempdir().unwrap();
    let (app, owner_id) = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sala/scope?u=pub-uni,priv-uni")
                .header(header::AUTHORIZATION, bearer(&owner_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(
        json["scope"], "priv-uni,pub-uni",
        "scope is normalized/sorted"
    );
    let mut keys = scope_keys(&json);
    keys.sort();
    assert_eq!(keys, vec!["priv-uni", "pub-uni"]);
}

/// An anonymous caller on the same subset sees ONLY the public universe — the
/// private one is filtered out by the visibility gate.
#[tokio::test]
async fn anon_subset_scope_sees_only_public() {
    let dir = tempdir().unwrap();
    let (app, _owner) = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sala/scope?u=pub-uni,priv-uni")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(
        scope_keys(&json),
        vec!["pub-uni"],
        "anon sees only public scope"
    );
}

/// The per-universe entry endpoints back the picker: the owner can read entries
/// from both in-scope universes (nodes from both), the anon only from the public.
#[tokio::test]
async fn entries_back_the_picker_per_visibility() {
    let dir = tempdir().unwrap();
    let (app, owner_id) = build_test_router(dir.path());

    // Owner: both universes return their entry.
    for key in ["pub-uni", "priv-uni"] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/universes/{key}/entries"))
                    .header(header::AUTHORIZATION, bearer(&owner_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "owner reads {key}");
        let json = body_json(resp).await;
        assert!(
            json["total"].as_i64().unwrap_or(0) >= 1,
            "{key} must expose its seeded entry to the picker"
        );
    }

    // Anon: private universe is gated (no nodes leak from it).
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/priv-uni/entries")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "anon must not read private-universe entries"
    );
}

/// A scope-keyed layout — including a cross-universe edge in `key::path`
/// notation — round-trips through PUT then GET for the owner.
#[tokio::test]
async fn scope_state_round_trips_cross_universe_edge() {
    let dir = tempdir().unwrap();
    let (app, owner_id) = build_test_router(dir.path());

    let layout = serde_json::json!({
        "v": 2,
        "notes": [
            { "id": "pub-uni::notas/hello.md", "entry_path": "pub-uni::notas/hello.md", "universe": "pub-uni", "x": 0, "y": 0 },
            { "id": "priv-uni::notas/secret.md", "entry_path": "priv-uni::notas/secret.md", "universe": "priv-uni", "x": 1, "y": 0 }
        ],
        "edges": [
            { "from": "pub-uni::notas/hello.md", "to": "priv-uni::notas/secret.md", "type": "rel" }
        ],
        "view": { "tx": 0, "ty": 0, "scale": 1 }
    });
    let put_body = serde_json::json!({ "layout_json": layout.to_string() });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/sala/state?scope=pub-uni,priv-uni")
                .header(header::AUTHORIZATION, bearer(&owner_id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(put_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let saved = body_json(resp).await;
    assert_eq!(saved["scope"], "priv-uni,pub-uni");

    // GET the same scope (order-independent — normalized) returns the layout.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sala/state?scope=priv-uni,pub-uni")
                .header(header::AUTHORIZATION, bearer(&owner_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let got = body_json(resp).await;
    let stored_layout: serde_json::Value =
        serde_json::from_str(got["layout_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        stored_layout["edges"][0]["from"], "pub-uni::notas/hello.md",
        "cross-universe edge persists in key::path notation"
    );
    assert_eq!(stored_layout["edges"][0]["to"], "priv-uni::notas/secret.md");
}

/// A multi-universe scope state can be shared: the token resolves to a state
/// whose `scope` is the multi-universe identifier (read-only, visibility is
/// re-checked by the scope resolver at read time).
#[tokio::test]
async fn multi_scope_share_token_carries_scope() {
    let dir = tempdir().unwrap();
    let (app, owner_id) = build_test_router(dir.path());

    // Save a scope state first (share requires an existing state).
    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/sala/state?scope=pub-uni,priv-uni")
                .header(header::AUTHORIZATION, bearer(&owner_id))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"layout_json":"{\"notes\":[]}"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::OK);

    // Mint a share token.
    let share = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/sala/state/share?scope=pub-uni,priv-uni")
                .header(header::AUTHORIZATION, bearer(&owner_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(share.status(), StatusCode::OK);
    let token = body_json(share).await["share_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!token.is_empty());

    // Resolve the token anonymously — read-only, and it carries the scope.
    let resolved = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/workspace-states/{token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::OK);
    let json = body_json(resolved).await;
    assert_eq!(json["scope"], "priv-uni,pub-uni");
}

/// `/api/v1/sala/scope` with no `u` (the bare `/sala` view) resolves to the
/// caller-visible universe set — public-only for an anonymous caller.
#[tokio::test]
async fn all_scope_anon_lists_public_only() {
    let dir = tempdir().unwrap();
    let (app, _owner) = build_test_router(dir.path());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/sala/scope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["scope"], "*");
    let keys = scope_keys(&json);
    assert!(
        keys.contains(&"pub-uni".to_string()),
        "public universe in all-scope"
    );
    assert!(
        !keys.contains(&"priv-uni".to_string()),
        "private universe must not appear in an anon all-scope"
    );
}
