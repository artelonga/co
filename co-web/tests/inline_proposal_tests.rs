//! Integration tests for `POST /api/v1/universes/:slug/proposals/inline`.
//!
//! Scenario 3 from the editing matrix: a logged-in user wants to
//! change content in a public universe they don't own. PUT on the
//! entry returns 403 (owner check); the SPA falls back to this
//! endpoint to land the change in the target universe's
//! `_proposals/` inbox.

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

fn test_bearer() -> String {
    let (token, _) = co_web::auth::sign_jwt(
        "test-user",
        "test@example.com",
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
    // The endpoint requires the target universe to exist; seed it.
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
            worker_supervisor: co_web::infra::workers::InProcessExecutor::new_arc(),
        }),
    });
    // CO-220: subscribe before spawning so events published synchronously
    // during the HTTP handler are not missed by the task.
    let mut notif_rx = state
        .core
        .event_bus
        .subscribe(co_web::events::EventFilter::Notification);
    let notif_state = state.clone();
    tokio::spawn(async move {
        while let Some(event) = notif_rx.recv().await {
            if let co_web::events::DomainEvent::NotificationRequested {
                recipient_id,
                kind,
                universe_key,
                room_id,
                actor_id,
                object_id,
                summary_key,
                summary_params,
            } = event
            {
                let storage = notif_state.core.storage.lock();
                let _ = storage.create_notification(
                    &recipient_id,
                    &kind,
                    universe_key.as_deref(),
                    room_id.as_deref(),
                    &actor_id,
                    &object_id,
                    &summary_key,
                    summary_params,
                );
            }
        }
    });
    build_router(state, None)
}

async fn body_to_json(body: Body) -> Value {
    let bytes = body.collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn inline_proposal_lands_in_target_proposals_folder() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    // template universe is seeded by seed_data; any authed user can
    // propose into it even though they don't own it.
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes/template/proposals/inline")
        .header(header::AUTHORIZATION, test_bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "target_path": "content/sobre.md",
                "body": "# Proposed rewrite\n\nThis is the proposed body.",
                "note": "tiny tweak",
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body = body_to_json(res.into_body()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "POST inline proposal should succeed; body: {body}"
    );

    let proposal_path = body["proposal_path"].as_str().expect("proposal_path");
    assert!(
        proposal_path.starts_with("_proposals/"),
        "proposal must land under _proposals/, got: {proposal_path}"
    );
    assert!(
        proposal_path.ends_with(".md"),
        "proposal path must end .md, got: {proposal_path}"
    );
    assert_eq!(body["target_universe"], "template");
    assert_eq!(body["target_path"], "content/sobre.md");
    assert_eq!(body["author"], "test-user");
    assert_eq!(body["status"], "open");

    // Verify the entry is fetchable from the target universe.
    // Path segments only contain timestamps + author id + nanoid +
    // ".md" — no chars that need URL-encoding — so a plain join works.
    let encoded = proposal_path.to_string();
    let fetch_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/universes/template/entries/{encoded}"))
        .body(Body::empty())
        .unwrap();
    let fetch_res = app.clone().oneshot(fetch_req).await.unwrap();
    assert_eq!(fetch_res.status(), StatusCode::OK);
    let entry = body_to_json(fetch_res.into_body()).await;
    let fm = &entry["frontmatter"];
    assert_eq!(fm["type"], "proposal");
    assert_eq!(fm["kind"], "inline");
    assert_eq!(fm["target_path"], "content/sobre.md");
    assert_eq!(fm["target_universe"], "template");
    assert_eq!(fm["status"], "open");
    assert_eq!(fm["author"], "test-user");
    assert_eq!(fm["note"], "tiny tweak");
}

#[tokio::test]
async fn inline_proposal_requires_auth() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes/template/proposals/inline")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"target_path": "content/sobre.md", "body": "x"}).to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn inline_proposal_rejects_traversal_in_target_path() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes/template/proposals/inline")
        .header(header::AUTHORIZATION, test_bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"target_path": "../../etc/passwd", "body": "x"}).to_string(),
        ))
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

/// Helper: create a user + a universe they own. Required because
/// `user_notifications` has a FK on `users(id)` — notifications about
/// the owner silently fail if the user row is missing.
fn seed_owned_universe(dir: &std::path::Path, owner_id: &str, slug: &str) {
    let storage = Storage::new(dir);
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO users (id, email, display_name, created_at) \
             VALUES (?1, ?2, ?3, '2026-01-01')",
            rusqlite::params![
                owner_id,
                format!("{owner_id}@test.local"),
                format!("Test {owner_id}")
            ],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_public, visibility) \
             VALUES (?1, ?2, 'owned by test', ?3, '2026-01-01', 1, 'public-subscribable')",
            rusqlite::params![slug, format!("Owned-{slug}"), owner_id],
        )
        .unwrap();
}

#[tokio::test]
async fn inline_proposal_notifies_owner() {
    // Create a universe owned by another user; an authed third party
    // submits an inline proposal; the owner gets a notification.
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "owner-uid", "owned-universe");
    let app = build_test_router(dir.path());

    // Test user submits a proposal into "owned-universe".
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes/owned-universe/proposals/inline")
        .header(header::AUTHORIZATION, test_bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "target_path": "x.md",
                "body": "proposed body"
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // CO-220: yield to let the notification event bus listener process the
    // NotificationRequested event before reading storage.
    tokio::task::yield_now().await;

    // Look up the owner's notifications via storage directly (the
    // notifications API endpoint requires its own auth setup; storage
    // is sufficient to confirm the row landed).
    let storage = Storage::new(dir.path());
    let (notifs, _) = storage.list_my_notifications("owner-uid", None, 10);
    assert!(
        notifs.iter().any(|n| n.event_type == "universe.proposal"
            && n.context.universe_key.as_deref() == Some("owned-universe")),
        "owner should have received a universe.proposal notification"
    );
}

#[tokio::test]
async fn decide_merge_writes_target_and_flips_status() {
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());
    seed_owned_universe(dir.path(), "test-user", "my-universe");

    // Create a proposal (test-user is also the owner — they decide on their own).
    let create_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/my-universe/proposals/inline")
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"target_path": "merged.md", "body": "the merged body"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_res.status(), StatusCode::OK);
    let payload = body_to_json(create_res.into_body()).await;
    let proposal_path = payload["proposal_path"].as_str().unwrap().to_string();

    // Decide: merge.
    let decide_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/my-universe/proposals/decide")
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"proposal_path": proposal_path, "action": "merge"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let decide_status = decide_res.status();
    let decide_body = body_to_json(decide_res.into_body()).await;
    assert_eq!(
        decide_status,
        StatusCode::OK,
        "merge should succeed; body: {decide_body}"
    );
    assert_eq!(decide_body["status"], "merged");

    // Target entry should now have the merged body.
    let fetch_res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/my-universe/entries/merged.md")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetch_res.status(), StatusCode::OK);
    let entry = body_to_json(fetch_res.into_body()).await;
    assert_eq!(entry["body"], "the merged body");
}

#[tokio::test]
async fn decide_requires_universe_owner() {
    let dir = tempdir().unwrap();
    // Universe owned by someone other than test-user.
    seed_owned_universe(dir.path(), "other-owner", "other-universe");
    let app = build_test_router(dir.path());

    // test-user proposes (allowed).
    let create_res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/other-universe/proposals/inline")
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"target_path": "x.md", "body": "x"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_res.status(), StatusCode::OK);
    let proposal_path = body_to_json(create_res.into_body()).await["proposal_path"]
        .as_str()
        .unwrap()
        .to_string();

    // test-user tries to decide (should fail — not the owner).
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes/other-universe/proposals/decide")
                .header(header::AUTHORIZATION, test_bearer())
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"proposal_path": proposal_path, "action": "merge"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn inbox_lists_proposals_for_owned_universes_only() {
    let dir = tempdir().unwrap();
    seed_owned_universe(dir.path(), "test-user", "mine");
    seed_owned_universe(dir.path(), "other-owner", "theirs");
    let app = build_test_router(dir.path());

    // Submit one proposal to each universe.
    for slug in ["mine", "theirs"] {
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/universes/{slug}/proposals/inline"))
                    .header(header::AUTHORIZATION, test_bearer())
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({"target_path": format!("for-{slug}.md"), "body": "x"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    // Inbox for test-user: only "mine" proposals show up.
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/me/inbound-proposals")
                .header(header::AUTHORIZATION, test_bearer())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_to_json(res.into_body()).await;
    let proposals = body["proposals"].as_array().unwrap();
    assert_eq!(
        proposals.len(),
        1,
        "inbox should contain only owned-universe proposals; got: {body}"
    );
    assert_eq!(proposals[0]["universe"], "mine");
}

#[tokio::test]
async fn inline_proposal_server_overrides_smuggled_frontmatter() {
    // The caller can only set body, target_path, and (optional) note.
    // Even if they tried to send extra frontmatter to forge author or
    // status, the endpoint accepts only the three known fields — so
    // this test confirms the request schema rejects unknown ones, or
    // (if it ignores them) the resulting entry still has the right
    // server-controlled author/status.
    let dir = tempdir().unwrap();
    let app = build_test_router(dir.path());

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/universes/template/proposals/inline")
        .header(header::AUTHORIZATION, test_bearer())
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "target_path": "content/sobre.md",
                "body": "x",
                "author": "attacker",            // ignored
                "status": "merged",              // ignored
                "frontmatter": {"type": "page"}, // ignored
            })
            .to_string(),
        ))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_to_json(res.into_body()).await;
    assert_eq!(body["author"], "test-user", "author is server-set");
    assert_eq!(body["status"], "open", "status is server-set");
}
