// CO-262: entries seeded by CO-261 walker must be visible to anonymous visitors
// via GET /api/v1/universes/co/entries.
// CO-266: total must reflect the full visible count; items must be the paginated slice.
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

use co_web::config::WebConfig;
use co_web::experiment::ExperimentStore;
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
        gestao_github_admins: vec!["artelonga".to_string()],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        quilombo_legacy_login: true,
        bypass_rate_limit: false,
    }
}

fn build_co_app(dir: &std::path::Path, seed_dir: &std::path::Path) -> axum::Router {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);
    storage.seed_admin_content_universes();
    storage.seed_co_universe_tasks(seed_dir);
    let experiment = ExperimentStore::new(&config.data_dir);
    let auth_store = co_web::auth::AuthStore::new(dir).unwrap();
    let mail: std::sync::Arc<dyn co::MailProvider> = std::sync::Arc::new(co::LogMailProvider);
    let game_db_path = dir.join("game_test.db");
    let game_storage = std::sync::Arc::new(
        game_core::storage::Storage::open(&game_db_path).expect("open test game storage"),
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

fn make_co_task_md(n: u64, status: &str) -> String {
    format!(
        "---\nid: {n}\ntitle: Test CO-{n}\ntype: user-story\nstatus: {status}\n\
         priority: high\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-02-01T00:00:00Z\n\
         labels:\n  - type:feat\n---\n\nBody of CO-{n}.",
    )
}

#[tokio::test]
async fn test_co_entries_visible_to_anon_after_seed() {
    let dir = tempdir().unwrap();
    let seed_dir = tempdir().unwrap();
    std::fs::write(seed_dir.path().join("CO-1.md"), make_co_task_md(1, "todo")).unwrap();
    std::fs::write(seed_dir.path().join("CO-2.md"), make_co_task_md(2, "done")).unwrap();

    let app = build_co_app(dir.path(), seed_dir.path());

    // Anonymous GET — no Authorization header
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co/entries?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let total = json["total"].as_u64().unwrap_or(0);
    assert!(
        total > 0,
        "anonymous visitor should see CO task entries via entries API, got total={total}"
    );
    let entries = json["entries"].as_array().expect("entries array");
    assert!(
        !entries.is_empty(),
        "entries array must be non-empty for anonymous visitor"
    );

    // Single-entry lookup must resolve to 200 for the public/ path
    let single = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co/entries/public/CO-1.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        single.status(),
        StatusCode::OK,
        "GET .../entries/public/CO-1.md must return 200 for anon visitor"
    );
}

/// CO-266: total must equal the full visible count while items is capped at the
/// requested limit.  Seeds 5 task entries, requests limit=3.
///
/// CO-268: since `co` is public-subscribable, `seed_co_universe_tasks` also
/// inserts `projects/CO/_project.md` which is now visible to anonymous callers.
/// Total therefore equals 5 tasks + 1 project = 6. Items is capped at limit=3.
#[tokio::test]
async fn test_list_entries_total_equals_n_items_equals_min_n_limit() {
    let dir = tempdir().unwrap();
    let seed_dir = tempdir().unwrap();
    for i in 1u64..=5 {
        std::fs::write(
            seed_dir.path().join(format!("CO-{i}.md")),
            make_co_task_md(i, "todo"),
        )
        .unwrap();
    }

    let app = build_co_app(dir.path(), seed_dir.path());

    // limit=3, 5 task entries + 1 project entry → total=6, items.length=3
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co/entries?limit=3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let total = json["total"].as_u64().expect("total field");
    let items = json["entries"].as_array().expect("entries array");
    assert!(
        total >= 5,
        "total must be at least 5 (the seeded task entries), got {total}"
    );
    assert_eq!(
        items.len(),
        3,
        "items must equal min(total, limit=3)=3, got {}",
        items.len()
    );
    assert!(
        total > items.len() as u64,
        "total ({total}) must exceed items.len() ({}) to prove pagination is not capping the count",
        items.len()
    );

    // path_prefix=public/ variant: only the 5 task entries (under public/) are
    // returned — the project entry (projects/CO/_project.md) is not under public/.
    let resp2 = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co/entries?path_prefix=public%2F&limit=3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp2.status(), StatusCode::OK);
    let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();
    let json2: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();

    let total2 = json2["total"].as_u64().expect("total field");
    let items2 = json2["entries"].as_array().expect("entries array");
    assert_eq!(
        total2, 5,
        "path_prefix=public/ total must equal 5 task entries, got {total2}"
    );
    assert_eq!(
        items2.len(),
        3,
        "path_prefix items must equal min(5, limit=3)=3, got {}",
        items2.len()
    );
}

/// CO-268: public-subscribable universes must expose ALL entries to anonymous
/// callers, not just those under `public/`. `filter_public_for_anon` must not
/// apply the per-path restriction when the universe visibility is
/// `public-subscribable` — the CO-161 visibility gate already controls access
/// at the universe level.
///
/// Seeds only the CO project entry (`projects/CO/_project.md`), which is NOT
/// under `public/`. Before the fix, anonymous callers got total=0, entries=[].
/// After the fix, they get total>0, entries not empty.
#[tokio::test]
async fn test_co268_anon_sees_all_entries_in_public_subscribable_universe() {
    let dir = tempdir().unwrap();
    let seed_dir = tempdir().unwrap(); // empty — no CO-N.md task files

    // seed_co_universe_tasks always inserts projects/CO/_project.md even when
    // seed_dir is empty. That entry is NOT under public/.
    let app = build_co_app(dir.path(), seed_dir.path());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co/entries?limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    let total = json["total"].as_u64().unwrap_or(0);
    let entries = json["entries"].as_array().expect("entries array");

    assert!(
        total > 0,
        "total must be > 0 for public-subscribable co universe (got {total}); \
         projects/CO/_project.md should be visible to anonymous callers"
    );
    assert!(
        !entries.is_empty(),
        "entries must not be empty for anon user on public-subscribable universe; \
         filter_public_for_anon must not restrict public-subscribable universes"
    );
    assert_eq!(
        total,
        entries.len() as u64,
        "total must equal items.len() when total < limit, got total={total} items={}",
        entries.len()
    );

    // Also verify the single-entry GET for a non-public path works for anon.
    let single = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/co/entries/projects/CO/_project.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        single.status(),
        StatusCode::OK,
        "GET .../entries/projects/CO/_project.md must return 200 for anon on public-subscribable universe"
    );
}
