//! CO-368: per-universe Scrum API.
//!
//! These endpoints make Scrum *data*, not a separate tool: PBIs and Sprints are
//! ordinary CO entries (`entry_type = "pbi" | "sprint"`) read/written through
//! the per-universe [`EntryStore`] seam (CO-433), and the cadence is computed
//! from the universe's `_scrum.yaml`. Universes without a manifest report
//! `enabled: false` and are otherwise untouched.
//!
//! Mounted under `/api/v1/universes` (see `server/router.rs`).
//!
//! [`EntryStore`]: crate::repository::EntryStore

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::atividade::{Acao, Atividade, Tipo, log_atividade};
use crate::domain::EntryDomain;
use crate::eda::{Event, Visibility};
use crate::repository::EntryStore;
use crate::server::AppState;
use crate::server::state::lock_storage;

use super::manifest::load_scrum;
use super::validate::validate_pbi;

const PBI_DIR: &str = "scrum/pbi";

// ---------------------------------------------------------------------------
// Key validation (mirrors time/routes.rs — the fallback content root joins the
// raw key, so a traversal slug must be rejected before any filesystem access).
// ---------------------------------------------------------------------------

fn is_valid_universe_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// `id` path segment for a PBI — a single slug, no slashes/traversal.
fn is_valid_pbi_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_' || b == b'.'
        })
        && !s.contains("..")
}

fn store_for(state: &AppState, key: &str) -> Result<Arc<dyn EntryStore>, (StatusCode, String)> {
    let storage = lock_storage(state);
    storage.entry_store(key).map_err(|e| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("scrum: cannot open universe store: {e}"),
        )
    })
}

// ---------------------------------------------------------------------------
// GET /{key}/scrum/manifest
// ---------------------------------------------------------------------------

async fn get_manifest(
    State(state): State<AppState>,
    AxumPath(key): AxumPath<String>,
) -> impl IntoResponse {
    if !is_valid_universe_key(&key) {
        return (StatusCode::BAD_REQUEST, "invalid universe key").into_response();
    }
    let root = crate::workspace_template_routes::resolve_universe_content_root(&state, &key);
    let manifest = load_scrum(&root);
    let current = manifest.current_at(chrono::Utc::now());
    let mut body = serde_json::to_value(&manifest).unwrap_or_else(|_| json!({}));
    body["current_sprint"] = serde_json::to_value(current).unwrap_or(Value::Null);
    Json(body).into_response()
}

// ---------------------------------------------------------------------------
// GET /{key}/scrum/sprints/current
// ---------------------------------------------------------------------------

async fn get_current_sprint(
    State(state): State<AppState>,
    AxumPath(key): AxumPath<String>,
) -> impl IntoResponse {
    if !is_valid_universe_key(&key) {
        return (StatusCode::BAD_REQUEST, "invalid universe key").into_response();
    }
    let root = crate::workspace_template_routes::resolve_universe_content_root(&state, &key);
    let manifest = load_scrum(&root);
    match manifest.current_at(chrono::Utc::now()) {
        Some(cur) => Json(cur).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            "scrum not enabled or missing sprint_start_anchor",
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /{key}/scrum/sprints — list sprint entries (sorted by number)
// ---------------------------------------------------------------------------

async fn list_sprints(
    State(state): State<AppState>,
    AxumPath(key): AxumPath<String>,
) -> impl IntoResponse {
    if !is_valid_universe_key(&key) {
        return (StatusCode::BAD_REQUEST, "invalid universe key").into_response();
    }
    let store = match store_for(&state, &key) {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let mut entries = match store.list("sprint", &Value::Null, None) {
        Ok(e) => e,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    entries.sort_by_key(|e| sprint_number(&e.frontmatter));
    let out: Vec<Value> = entries.iter().map(entry_to_json).collect();
    Json(json!({ "sprints": out, "total": out.len() })).into_response()
}

fn sprint_number(fm: &Value) -> i64 {
    fm.get("number").and_then(Value::as_i64).unwrap_or(i64::MAX)
}

// ---------------------------------------------------------------------------
// GET /{key}/scrum/backlog?status=ready — filtered PBI list
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct BacklogQuery {
    status: Option<String>,
    sprint: Option<String>,
}

async fn get_backlog(
    State(state): State<AppState>,
    AxumPath(key): AxumPath<String>,
    Query(q): Query<BacklogQuery>,
) -> impl IntoResponse {
    if !is_valid_universe_key(&key) {
        return (StatusCode::BAD_REQUEST, "invalid universe key").into_response();
    }
    let store = match store_for(&state, &key) {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let entries = match store.list("pbi", &Value::Null, None) {
        Ok(e) => e,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let out: Vec<Value> = entries
        .iter()
        .filter(|e| {
            q.status
                .as_deref()
                .is_none_or(|s| e.frontmatter.get("status").and_then(Value::as_str) == Some(s))
        })
        .filter(|e| {
            q.sprint
                .as_deref()
                .is_none_or(|s| e.frontmatter.get("sprint").and_then(Value::as_str) == Some(s))
        })
        .map(entry_to_json)
        .collect();
    Json(json!({ "pbis": out, "total": out.len() })).into_response()
}

// ---------------------------------------------------------------------------
// POST /{key}/scrum/pbi — create a PBI (sugar over an entry write)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CreatePbiBody {
    id: String,
    title: String,
    priority: String,
    #[serde(default)]
    points: Option<Value>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    acceptance: Vec<String>,
    #[serde(default)]
    sprint: Option<String>,
    #[serde(default)]
    assignees: Vec<String>,
    #[serde(default)]
    epic: Option<String>,
    #[serde(default)]
    body: String,
}

async fn create_pbi(
    State(state): State<AppState>,
    AxumPath(key): AxumPath<String>,
    headers: HeaderMap,
    Json(req): Json<CreatePbiBody>,
) -> impl IntoResponse {
    if !is_valid_universe_key(&key) {
        return (StatusCode::BAD_REQUEST, "invalid universe key").into_response();
    }
    if !is_valid_pbi_id(&req.id) {
        return (StatusCode::BAD_REQUEST, "invalid pbi id").into_response();
    }

    let status = req.status.clone().unwrap_or_else(|| "backlog".to_string());
    let mut fm = json!({
        "type": "pbi",
        "title": req.title,
        "priority": req.priority,
        "status": status,
        "acceptance": req.acceptance,
        "assignees": req.assignees,
    });
    if let Some(p) = req.points {
        fm["points"] = p;
    }
    if let Some(s) = req.sprint {
        fm["sprint"] = json!(s);
    }
    if let Some(e) = req.epic {
        fm["epic"] = json!(e);
    }

    if let Err(msg) = validate_pbi(&fm) {
        return (StatusCode::UNPROCESSABLE_ENTITY, msg).into_response();
    }

    let path = format!("{PBI_DIR}/{}.md", req.id);
    let entry = make_domain(&key, &path, "pbi", fm, &req.body);

    let store = match store_for(&state, &key) {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = store.upsert(&entry) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    let user_id = crate::auth::resolve_user_id(&state, &headers);
    publish_scrum_event(
        &state,
        "scrum.pbi.created",
        &key,
        user_id.clone(),
        json!({ "path": path }),
    );
    log_atividade(
        state.clone(),
        Atividade {
            acao: Acao::Criar,
            entidade: "scrum.pbi".into(),
            entidade_id: Some(path.clone()),
            before: None,
            after: Some(entry.frontmatter.clone()),
            tipo: Tipo::Sucesso,
            user_id,
            ip: None,
            user_agent: None,
        },
    );

    (StatusCode::CREATED, Json(entry_to_json(&entry))).into_response()
}

// ---------------------------------------------------------------------------
// PATCH /{key}/scrum/pbi/{id}/dod — check off a DoD item
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DodPatchBody {
    index: usize,
    done: bool,
}

/// One DoD checklist item as stored on a PBI's frontmatter `dod` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DodItem {
    text: String,
    #[serde(default)]
    done: bool,
}

async fn patch_dod(
    State(state): State<AppState>,
    AxumPath((key, id)): AxumPath<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<DodPatchBody>,
) -> impl IntoResponse {
    if !is_valid_universe_key(&key) {
        return (StatusCode::BAD_REQUEST, "invalid universe key").into_response();
    }
    if !is_valid_pbi_id(&id) {
        return (StatusCode::BAD_REQUEST, "invalid pbi id").into_response();
    }
    let path = format!("{PBI_DIR}/{id}.md");

    let store = match store_for(&state, &key) {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let mut entry = match store.get(&path) {
        Ok(Some(e)) => e,
        Ok(None) => return (StatusCode::NOT_FOUND, "PBI not found").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    // The checklist lives on the PBI's `dod` frontmatter. If absent, seed it
    // from the universe `_scrum.yaml` default DoD on first check-off.
    let mut dod = read_dod(&entry.frontmatter);
    if dod.is_empty() {
        let root = crate::workspace_template_routes::resolve_universe_content_root(&state, &key);
        dod = load_scrum(&root)
            .default_dod
            .into_iter()
            .map(|text| DodItem { text, done: false })
            .collect();
    }
    if req.index >= dod.len() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("dod index {} out of range (len {})", req.index, dod.len()),
        )
            .into_response();
    }
    dod[req.index].done = req.done;

    entry.frontmatter["dod"] = serde_json::to_value(&dod).unwrap_or(Value::Null);
    if let Err(e) = store.upsert(&entry) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }

    let user_id = crate::auth::resolve_user_id(&state, &headers);
    publish_scrum_event(
        &state,
        "scrum.pbi.dod_checked",
        &key,
        user_id.clone(),
        json!({ "path": path, "index": req.index, "done": req.done }),
    );
    log_atividade(
        state.clone(),
        Atividade {
            acao: Acao::Atualizar,
            entidade: "scrum.pbi.dod".into(),
            entidade_id: Some(path.clone()),
            before: None,
            after: Some(json!({ "index": req.index, "done": req.done })),
            tipo: Tipo::Sucesso,
            user_id,
            ip: None,
            user_agent: None,
        },
    );

    Json(json!({ "path": path, "dod": dod })).into_response()
}

fn read_dod(fm: &Value) -> Vec<DodItem> {
    fm.get("dod")
        .and_then(|v| serde_json::from_value::<Vec<DodItem>>(v.clone()).ok())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_domain(key: &str, path: &str, entry_type: &str, fm: Value, body: &str) -> EntryDomain {
    let title = fm.get("title").and_then(Value::as_str).map(str::to_string);
    EntryDomain {
        path: path.to_string(),
        universe_key: key.to_string(),
        entry_type: entry_type.to_string(),
        title,
        frontmatter: fm,
        body: body.to_string(),
        body_hash: co::entry::Entry::hash_body(body),
        created_at: None,
        updated_at: None,
    }
}

fn entry_to_json(e: &EntryDomain) -> Value {
    json!({
        "path": e.path,
        "type": e.entry_type,
        "title": e.title_or_path(),
        "frontmatter": e.frontmatter,
        "body": e.body,
    })
}

fn publish_scrum_event(
    state: &AppState,
    event_type: &str,
    key: &str,
    user_id: Option<String>,
    payload: Value,
) {
    state.core.eda_bus.publish(Event::new(
        event_type,
        Some(key.to_string()),
        user_id,
        payload,
        Visibility::UniverseMembers,
    ));
}

pub fn universe_router() -> Router<AppState> {
    Router::new()
        .route("/{key}/scrum/manifest", get(get_manifest))
        .route("/{key}/scrum/sprints", get(list_sprints))
        .route("/{key}/scrum/sprints/current", get(get_current_sprint))
        .route("/{key}/scrum/backlog", get(get_backlog))
        .route("/{key}/scrum/pbi", post(create_pbi))
        .route("/{key}/scrum/pbi/{id}/dod", patch(patch_dod))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn build_test_router(dir: &std::path::Path) -> axum::Router {
        unsafe { std::env::set_var("JWT_SECRET", "test-secret") };
        let config = crate::config::WebConfig {
            data_dir: dir.to_str().unwrap().to_string(),
            port: 0,
            static_dir: String::new(),
            default_variant: "a".into(),
            experiments: false,
            plugins_dir: String::new(),
            game_db_path: None,
            universo_dir: String::new(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: "test".into(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            quilombo_legacy_login: true,
            bypass_rate_limit: true,
        };
        let storage = crate::storage::Storage::new(dir);
        let experiment = crate::experiment::ExperimentStore::new(dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
        let (embedding_tx, _rx) = crate::embedding_worker::channel();
        let state = crate::server::AppState::new(crate::server::AppStateInner {
            core: Arc::new(crate::server::CoreState::from_storage(
                storage, config, auth_store,
            )),
            realtime: Arc::new(crate::server::RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: Mutex::new(std::collections::HashMap::new()),
                chat_presence: Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(crate::server::IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(crate::server::IntegrationsState {
                mail,
                geo: Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        });
        crate::server::build_router(state, None)
    }

    /// Write `_scrum.yaml` into the universe's fallback content root.
    fn write_scrum_yaml(dir: &std::path::Path, key: &str, yaml: &str) {
        let root = dir.join("universes").join(key);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("_scrum.yaml"), yaml).unwrap();
    }

    const ENABLED_YAML: &str = r#"
enabled: true
sprint_length_days: 14
sprint_start_anchor: "2026-06-11T15:00:00-03:00"
release_tag_pattern: "v{major}.{minor}.0"
default_dod:
  - "Acceptance criteria green"
  - "cargo test clean"
"#;

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn get(app: &axum::Router, uri: &str) -> axum::response::Response {
        app.clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn send_json(
        app: &axum::Router,
        method: &str,
        uri: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// AC: a universe WITHOUT `_scrum.yaml` is unchanged — `enabled: false`,
    /// no current sprint.
    #[tokio::test]
    async fn manifest_disabled_when_absent() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = get(&app, "/api/v1/universes/plain-uni/scrum/manifest").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["enabled"], false);
        assert!(json["current_sprint"].is_null());
    }

    /// AC: `_scrum.yaml` loaded + exposed; current sprint computed.
    #[tokio::test]
    async fn manifest_enabled_exposes_current_sprint() {
        let dir = tempdir().unwrap();
        write_scrum_yaml(dir.path(), "co-dev", ENABLED_YAML);
        let app = build_test_router(dir.path());
        let resp = get(&app, "/api/v1/universes/co-dev/scrum/manifest").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["enabled"], true);
        assert_eq!(json["sprint_length_days"], 14);
        assert!(json["current_sprint"]["number"].as_i64().unwrap() >= 1);
        assert!(json["current_sprint"]["start_at"].is_string());
    }

    /// AC: `/sprints/current` returns `{number, start_at, end_at, release_window}`.
    #[tokio::test]
    async fn current_sprint_shape() {
        let dir = tempdir().unwrap();
        write_scrum_yaml(dir.path(), "co-dev", ENABLED_YAML);
        let app = build_test_router(dir.path());
        let resp = get(&app, "/api/v1/universes/co-dev/scrum/sprints/current").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["number"].is_number());
        assert!(json["start_at"].is_string());
        assert!(json["end_at"].is_string());
        assert!(json["release_window"].is_boolean());
    }

    /// `/sprints/current` is 404 when the universe has no Scrum manifest.
    #[tokio::test]
    async fn current_sprint_404_when_disabled() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = get(&app, "/api/v1/universes/plain-uni/scrum/sprints/current").await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// AC: create a PBI, then read it back via the backlog (with status filter).
    #[tokio::test]
    async fn create_pbi_and_filter_backlog() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = send_json(
            &app,
            "POST",
            "/api/v1/universes/co-dev/scrum/pbi",
            serde_json::json!({
                "id": "co-368",
                "title": "Scrum artifacts",
                "priority": "high",
                "status": "ready",
                "points": "M",
                "acceptance": ["manifest loads", "board renders"]
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        // Backlog filtered to ready → contains our PBI.
        let resp = get(&app, "/api/v1/universes/co-dev/scrum/backlog?status=ready").await;
        let json = body_json(resp).await;
        assert_eq!(json["total"], 1);
        assert_eq!(json["pbis"][0]["frontmatter"]["priority"], "high");

        // Filtered to a different status → empty.
        let resp = get(&app, "/api/v1/universes/co-dev/scrum/backlog?status=done").await;
        let json = body_json(resp).await;
        assert_eq!(json["total"], 0);
    }

    /// AC: PBI frontmatter is validated — a bad status is rejected (422).
    #[tokio::test]
    async fn create_pbi_rejects_bad_status() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = send_json(
            &app,
            "POST",
            "/api/v1/universes/co-dev/scrum/pbi",
            serde_json::json!({
                "id": "bad",
                "title": "Bad",
                "priority": "low",
                "status": "in-progress"
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// AC: DoD check-off — seeds from `default_dod`, toggles by index.
    #[tokio::test]
    async fn dod_check_off_seeds_and_toggles() {
        let dir = tempdir().unwrap();
        write_scrum_yaml(dir.path(), "co-dev", ENABLED_YAML);
        let app = build_test_router(dir.path());
        send_json(
            &app,
            "POST",
            "/api/v1/universes/co-dev/scrum/pbi",
            serde_json::json!({ "id": "co-368", "title": "X", "priority": "high", "status": "ready" }),
        )
        .await;

        let resp = send_json(
            &app,
            "PATCH",
            "/api/v1/universes/co-dev/scrum/pbi/co-368/dod",
            serde_json::json!({ "index": 0, "done": true }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["dod"][0]["done"], true);
        assert_eq!(json["dod"][0]["text"], "Acceptance criteria green");
        assert_eq!(json["dod"][1]["done"], false);
    }

    /// DoD check-off on a missing PBI is a 404.
    #[tokio::test]
    async fn dod_check_off_404_for_missing_pbi() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = send_json(
            &app,
            "PATCH",
            "/api/v1/universes/co-dev/scrum/pbi/nope/dod",
            serde_json::json!({ "index": 0, "done": true }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// A path-traversal slug is rejected before any filesystem resolution.
    #[tokio::test]
    async fn rejects_traversal_slug() {
        let dir = tempdir().unwrap();
        let app = build_test_router(dir.path());
        let resp = get(&app, "/api/v1/universes/..%2f..%2fetc/scrum/manifest").await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn pbi_id_charset() {
        assert!(super::is_valid_pbi_id("co-368"));
        assert!(super::is_valid_pbi_id("pbi_1.2"));
        assert!(!super::is_valid_pbi_id(""));
        assert!(!super::is_valid_pbi_id("../etc"));
        assert!(!super::is_valid_pbi_id("a/b"));
    }
}
