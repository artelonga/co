//! CO-472 — integration tests for the unified thread model.
//!
//! Comments compose with messages under ONE thread primitive: a *message is a
//! comment in a thread*. These tests drive the real router in-process (no
//! ports, `tower::ServiceExt::oneshot`) and cover every user path:
//!
//! - comment-on-entry creates + returns an anchored thread (GET/POST),
//! - a reply creates a recursive child thread,
//! - posting a comment publishes a `ChatEvent` (asserted via a broadcast
//!   subscriber, like the chat ws tests),
//! - a non-member is rejected (403) and a member is accepted,
//! - and the storage layer covers the subset-member + self-note scopes.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
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
        universo_dir: "universo".to_string(),
        gestao_github_admins: vec!["artelonga".to_string()],
        universe_key: None,
        co_env: "prod".into(),
        wae_endpoint: None,
        wae_api_key: None,
        cookie_domain: None,
        bypass_rate_limit: true,
    }
}

fn bearer(user: &str) -> String {
    let (token, _) = co_web::auth::sign_jwt(
        user,
        &format!("{user}@t.local"),
        "player",
        "dev-secret-change-me",
    )
    .unwrap();
    format!("Bearer {token}")
}

/// Seed an owner-owned public universe and add `members` as full members so
/// `resolve_role` returns a posting role for them.
fn seed_universe(dir: &std::path::Path, owner: &str, slug: &str, members: &[&str]) {
    let storage = Storage::new(dir);
    for u in std::iter::once(&owner).chain(members.iter()) {
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
                 VALUES (?1, ?2, ?3, '2026-01-01')",
                rusqlite::params![u, format!("{u}@t.local"), "Test"],
            )
            .unwrap();
    }
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_public, visibility) \
             VALUES (?1, ?2, '', ?3, '2026-01-01', 1, 'public-subscribable')",
            rusqlite::params![slug, format!("U-{slug}"), owner],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, 'owner', '2026-01-01')",
            rusqlite::params![slug, owner],
        )
        .unwrap();
    for m in members {
        storage
            .conn()
            .execute(
                "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
                 VALUES (?1, ?2, 'member', '2026-01-01')",
                rusqlite::params![slug, m],
            )
            .unwrap();
    }
}

fn build_test_router(dir: &std::path::Path) -> (axum::Router, AppState) {
    let config = test_config(dir);
    let mut storage = Storage::new(&config.data_dir);
    seed_data(&mut storage);
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
            rate_limiter: std::sync::Mutex::new(co_web::rate_limit::InProcessRateLimiter::new()),
            experiment: Mutex::new(experiment),
            worker_supervisor: co_web::infra::workers::InProcessExecutor::new_arc(),
        }),
    });
    (build_router(state.clone(), None), state)
}

async fn body_to_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

async fn create_entry(app: &axum::Router, slug: &str, owner: &str, path: &str) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{slug}/entries"))
                .header(header::AUTHORIZATION, bearer(owner))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"path": path, "frontmatter": {"title": "X"}, "body": "hi"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(res.status().is_success(), "create entry: {}", res.status());
}

// --- comment-on-entry: GET empty, POST creates thread, GET returns it --------

#[tokio::test]
async fn comment_on_entry_creates_and_returns_thread() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "owner", "cm", &[]);
    let (app, _state) = build_test_router(dir.path());
    create_entry(&app, "cm", "owner", "content/a.md").await;

    // Before any comment: empty view, 200, null thread id.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/cm/comments?path=content/a.md")
                .header(header::AUTHORIZATION, bearer("owner"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let j = body_to_json(res.into_body()).await;
    assert!(j["thread_id"].is_null(), "no thread before first comment");
    assert_eq!(j["comments"].as_array().unwrap().len(), 0);

    // POST a comment → 201, creates the anchored thread.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/cm/comments?path=content/a.md")
                .header(header::AUTHORIZATION, bearer("owner"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"body": "first comment"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED, "post comment");

    // GET now returns the anchored thread + the message.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/cm/comments?path=content/a.md")
                .header(header::AUTHORIZATION, bearer("owner"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let j = body_to_json(res.into_body()).await;
    assert!(!j["thread_id"].is_null(), "thread now exists");
    let comments = j["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0]["body"], "first comment");
}

// --- recursive reply: parent_id spawns a child thread ------------------------

#[tokio::test]
async fn reply_creates_recursive_child_thread() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "owner", "rp", &[]);
    let (app, _state) = build_test_router(dir.path());
    create_entry(&app, "rp", "owner", "content/a.md").await;

    // First comment.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/rp/comments?path=content/a.md")
                .header(header::AUTHORIZATION, bearer("owner"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"body": "root"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let parent_id = body_to_json(res.into_body()).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Reply referencing the parent message → recursive child thread.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/rp/comments?path=content/a.md")
                .header(header::AUTHORIZATION, bearer("owner"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"body": "a reply", "parent_id": parent_id}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // GET shows the reply under `replies` (a child thread), not the top level.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/rp/comments?path=content/a.md")
                .header(header::AUTHORIZATION, bearer("owner"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let j = body_to_json(res.into_body()).await;
    assert_eq!(j["comments"].as_array().unwrap().len(), 1, "1 top-level");
    let replies = j["replies"].as_array().unwrap();
    assert_eq!(replies.len(), 1, "one recursive child thread");
    assert_eq!(
        replies[0]["comments"].as_array().unwrap()[0]["body"],
        "a reply"
    );
}

// --- live delivery: posting publishes a ChatEvent over the broadcast bus -----

#[tokio::test]
async fn posting_a_comment_publishes_chat_event() {
    use co_web::chat::ChatBroadcast;
    use tokio::sync::broadcast;

    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "owner", "lv", &[]);
    let (app, state) = build_test_router(dir.path());
    create_entry(&app, "lv", "owner", "content/a.md").await;

    // Pre-create the anchored thread so we can subscribe to its channel before
    // the post (mirrors how a connected WS member already holds a subscription).
    let thread_id = {
        let storage = state.core.storage.lock();
        storage
            .get_or_create_anchored_thread("lv", "content/a.md", None, "owner")
            .unwrap()
            .id
    };
    let mut rx = {
        let mut map = state.realtime.chat_rooms_broadcast.lock().unwrap();
        let tx: &ChatBroadcast = map
            .entry(thread_id.clone())
            .or_insert_with(|| broadcast::channel::<Arc<str>>(64).0);
        tx.subscribe()
    };

    // POST a comment.
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/lv/comments?path=content/a.md")
                .header(header::AUTHORIZATION, bearer("owner"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"body": "live!"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // A ChatEvent::MessageCreated must have been fanned out.
    let raw = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("broadcast within 2s")
        .expect("a broadcast event");
    // ChatEvent is Serialize-only (the fan-out is serialize-once, CO-468), so
    // assert on the JSON wire form exactly as the chat ws tests do.
    let evt: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        evt["type"], "message.created",
        "fanned-out a MessageCreated"
    );
    assert_eq!(evt["message"]["body"], "live!");
}

// --- membership: non-member rejected (403), member accepted ------------------

#[tokio::test]
async fn non_member_rejected_member_accepted() {
    let dir = tempdir().unwrap();
    // "member" is a real member; "stranger" is not.
    seed_universe(dir.path(), "owner", "mb", &["member"]);
    let (app, _state) = build_test_router(dir.path());
    create_entry(&app, "mb", "owner", "content/a.md").await;

    // Member can post.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/mb/comments?path=content/a.md")
                .header(header::AUTHORIZATION, bearer("member"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"body": "hi from member"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED, "member may comment");

    // Stranger (not a member) is forbidden from posting.
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/mb/comments?path=content/a.md")
                .header(header::AUTHORIZATION, bearer("stranger"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"body": "let me in"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "non-member must be rejected"
    );
}

// ===========================================================================
// CO-476 — standalone scoped-thread REST routes (deferred by CO-472).
// ===========================================================================

async fn create_thread(
    app: &axum::Router,
    slug: &str,
    user: &str,
    scope: &str,
    members: &[&str],
) -> (StatusCode, Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/{slug}/threads"))
                .header(header::AUTHORIZATION, bearer(user))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"scope": scope, "members": members}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    (status, body_to_json(res.into_body()).await)
}

async fn post_thread_message(
    app: &axum::Router,
    slug: &str,
    user: &str,
    thread_id: &str,
    body: Value,
) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/v1/universes/{slug}/threads/{thread_id}/messages"
                ))
                .header(header::AUTHORIZATION, bearer(user))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn create_all_subset_self_threads_and_list() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "owner", "th", &["member"]);
    let (app, _state) = build_test_router(dir.path());

    // scope: all
    let (st, all) = create_thread(&app, "th", "owner", "all", &[]).await;
    assert_eq!(st, StatusCode::CREATED, "create all-scope thread");
    assert_eq!(all["scope"], "universe");

    // scope: subset with member
    let (st, subset) = create_thread(&app, "th", "owner", "subset", &["member"]).await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(subset["scope"], "subset");

    // scope: self
    let (st, selfn) = create_thread(&app, "th", "owner", "self", &[]).await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(selfn["scope"], "self");

    // owner lists all three.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/th/threads")
                .header(header::AUTHORIZATION, bearer("owner"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let j = body_to_json(res.into_body()).await;
    assert_eq!(
        j["threads"].as_array().unwrap().len(),
        3,
        "owner sees all 3"
    );

    // member sees the universe + subset thread, NOT the owner's self note.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/th/threads")
                .header(header::AUTHORIZATION, bearer("member"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let j = body_to_json(res.into_body()).await;
    assert_eq!(
        j["threads"].as_array().unwrap().len(),
        2,
        "member excluded from owner's self note"
    );
}

#[tokio::test]
async fn subset_thread_non_member_forbidden() {
    let dir = tempdir().unwrap();
    // member is in the subset; stranger is a universe member but NOT in the subset.
    seed_universe(dir.path(), "owner", "su", &["member", "stranger"]);
    let (app, _state) = build_test_router(dir.path());

    let (_st, subset) = create_thread(&app, "su", "owner", "subset", &["member"]).await;
    let tid = subset["id"].as_str().unwrap();

    // member (in subset) can post.
    assert_eq!(
        post_thread_message(&app, "su", "member", tid, json!({"body": "hi"})).await,
        StatusCode::CREATED,
        "subset member may post"
    );

    // stranger (not in subset, even though a universe member) is forbidden.
    assert_eq!(
        post_thread_message(&app, "su", "stranger", tid, json!({"body": "let me in"})).await,
        StatusCode::FORBIDDEN,
        "non-subset-member rejected"
    );

    // stranger cannot even GET the subset thread.
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/su/threads/{tid}"))
                .header(header::AUTHORIZATION, bearer("stranger"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "GET subset 403 for outsider"
    );
}

#[tokio::test]
async fn self_thread_private_to_creator() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "owner", "sf", &["member"]);
    let (app, _state) = build_test_router(dir.path());

    let (_st, selfn) = create_thread(&app, "sf", "owner", "self", &[]).await;
    let tid = selfn["id"].as_str().unwrap();

    // creator can post + read.
    assert_eq!(
        post_thread_message(&app, "sf", "owner", tid, json!({"body": "note to self"})).await,
        StatusCode::CREATED
    );
    // another universe member cannot access the self note.
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/sf/threads/{tid}"))
                .header(header::AUTHORIZATION, bearer("member"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN, "self note is private");
}

#[tokio::test]
async fn thread_post_and_recursive_reply() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "owner", "pr2", &[]);
    let (app, _state) = build_test_router(dir.path());

    let (_st, t) = create_thread(&app, "pr2", "owner", "all", &[]).await;
    let tid = t["id"].as_str().unwrap();

    // Top-level message.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/pr2/threads/{tid}/messages"))
                .header(header::AUTHORIZATION, bearer("owner"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"body": "root"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let parent_id = body_to_json(res.into_body()).await["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Reply → recursive child thread.
    assert_eq!(
        post_thread_message(
            &app,
            "pr2",
            "owner",
            tid,
            json!({"body": "a reply", "parent_id": parent_id})
        )
        .await,
        StatusCode::CREATED
    );

    // GET shows 1 top-level message + 1 recursive child thread under replies.
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/universes/pr2/threads/{tid}"))
                .header(header::AUTHORIZATION, bearer("owner"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let j = body_to_json(res.into_body()).await;
    assert_eq!(j["messages"].as_array().unwrap().len(), 1, "1 top-level");
    let replies = j["replies"].as_array().unwrap();
    assert_eq!(replies.len(), 1, "one recursive child thread");
    assert_eq!(
        replies[0]["messages"].as_array().unwrap()[0]["body"],
        "a reply"
    );
}

#[tokio::test]
async fn thread_archive_and_unarchive() {
    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "owner", "ar", &[]);
    let (app, _state) = build_test_router(dir.path());

    let (_st, t) = create_thread(&app, "ar", "owner", "all", &[]).await;
    let tid = t["id"].as_str().unwrap();

    // Archive.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/ar/threads/{tid}/archive"))
                .header(header::AUTHORIZATION, bearer("owner"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"archived": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        !body_to_json(res.into_body()).await["archived_at"].is_null(),
        "archived_at set"
    );

    // Default list excludes it.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/ar/threads")
                .header(header::AUTHORIZATION, bearer("owner"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let j = body_to_json(res.into_body()).await;
    assert_eq!(j["threads"].as_array().unwrap().len(), 0, "archived hidden");

    // include_archived=true returns it.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/ar/threads?include_archived=true")
                .header(header::AUTHORIZATION, bearer("owner"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let j = body_to_json(res.into_body()).await;
    assert_eq!(j["threads"].as_array().unwrap().len(), 1);

    // Un-archive.
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/universes/ar/threads/{tid}/archive"))
                .header(header::AUTHORIZATION, bearer("owner"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"archived": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        body_to_json(res.into_body()).await["archived_at"].is_null(),
        "archived_at cleared"
    );
}

#[tokio::test]
async fn posting_to_thread_publishes_chat_event() {
    use co_web::chat::ChatBroadcast;
    use tokio::sync::broadcast;

    let dir = tempdir().unwrap();
    seed_universe(dir.path(), "owner", "lvt", &[]);
    let (app, state) = build_test_router(dir.path());

    let (_st, t) = create_thread(&app, "lvt", "owner", "all", &[]).await;
    let tid = t["id"].as_str().unwrap().to_string();

    // Subscribe to the thread's broadcast channel before posting.
    let mut rx = {
        let mut map = state.realtime.chat_rooms_broadcast.lock().unwrap();
        let tx: &ChatBroadcast = map
            .entry(tid.clone())
            .or_insert_with(|| broadcast::channel::<Arc<str>>(64).0);
        tx.subscribe()
    };

    assert_eq!(
        post_thread_message(&app, "lvt", "owner", &tid, json!({"body": "live thread!"})).await,
        StatusCode::CREATED
    );

    let raw = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("broadcast within 2s")
        .expect("a broadcast event");
    let evt: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(evt["type"], "message.created");
    assert_eq!(evt["message"]["body"], "live thread!");
}
