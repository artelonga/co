//! CO-416: content-translation HTTP routes.
//!
//! - `POST /api/v1/universes/{slug}/translate/{*path}?to=en`
//!   Owner-gated. Generates a sibling translation of the entry at `path` in the
//!   target language. The twin is a **separate** entry at a parallel path
//!   (`foo.en.md` for source `foo.md`), carrying `translated_from` +
//!   `translated_from_hash` frontmatter so it is traceable and re-translation is
//!   idempotent by source-body hash. Returns 503 when no backend is configured.
//!
//! - `GET /api/v1/universes/{slug}/translation/{*path}`
//!   Surfaces whether a translation exists for `path` and, if so, its twin path
//!   and language — the data the UI language toggle needs to switch or to offer
//!   "generate".
//!
//! (The entry path is a catch-all, which Axum only allows at the end of a
//! route, so the action verb precedes the path rather than following it.)
//!
//! Structure preservation (code blocks, inline code, wikilinks, frontmatter
//! machine fields) lives in [`crate::translate`]; this module is the thin
//! controller (auth, path math, persistence, idempotency).

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::server::AppState;
use crate::translate::{self, TranslateBackend};

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TranslateQuery {
    /// Target language: `pt` or `en`. Defaults to the opposite of the source.
    pub to: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TranslateResponse {
    /// Path of the twin entry that holds the translation.
    pub path: String,
    /// Source entry path.
    pub source_path: String,
    /// Target language of the twin.
    pub lang: String,
    /// `"created"`, `"updated"`, or `"unchanged"` (idempotent no-op).
    pub status: String,
    /// Backend that produced the translation (`"llm"`, etc).
    pub backend: String,
}

#[derive(Debug, Serialize)]
pub struct TranslationInfo {
    /// Whether a twin translation exists for the requested entry.
    pub exists: bool,
    /// The twin's path, if it exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The twin's language, if it exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Whether the twin is stale (source body changed since translation).
    pub stale: bool,
}

// ---------------------------------------------------------------------------
// Path / lang helpers
// ---------------------------------------------------------------------------

/// Compute the twin path for `source_path` in `to_lang`.
///
/// `foo/bar.md` + `en` → `foo/bar.en.md`. If `source_path` already carries a
/// lang suffix (`foo/bar.pt.md`), it is stripped first so twins never stack
/// (`foo/bar.pt.md` + `en` → `foo/bar.en.md`).
pub fn twin_path(source_path: &str, to_lang: &str) -> String {
    let base = strip_lang_suffix(source_path);
    if let Some(stem) = base.strip_suffix(".md") {
        format!("{stem}.{to_lang}.md")
    } else {
        format!("{base}.{to_lang}")
    }
}

/// If `path` ends in `.<lang>.md` for a supported lang, return the path without
/// that lang segment (`foo/bar.en.md` → `foo/bar.md`). Otherwise unchanged.
pub fn strip_lang_suffix(path: &str) -> String {
    if let Some(stem) = path.strip_suffix(".md") {
        for lang in translate::LANGS {
            let suffix = format!(".{lang}");
            if let Some(without) = stem.strip_suffix(&suffix) {
                return format!("{without}.md");
            }
        }
    }
    path.to_string()
}

/// The source language: the entry's declared `lang`, or `pt` (canonical) when
/// absent — pt-br is the universe's default authoring language.
fn source_lang(frontmatter: &serde_json::Value) -> String {
    frontmatter
        .get("lang")
        .and_then(|v| v.as_str())
        .filter(|l| translate::is_supported_lang(l))
        .unwrap_or("pt")
        .to_string()
}

// ---------------------------------------------------------------------------
// Core: build the twin entry (pure, backend-injected — unit-tested directly)
// ---------------------------------------------------------------------------

/// The outcome of translating one source entry.
pub struct TwinOutcome {
    pub entry: co::entry::Entry,
    pub from_lang: String,
    pub to_lang: String,
}

/// Produce the twin `Entry` for `source` in `to_lang` using `backend`.
///
/// Carries provenance frontmatter:
/// - `lang: <to_lang>`
/// - `translated_from: <source.path>` (typed relation via `sync_entry_relations`)
/// - `translated_from_hash: <source.body_hash>` (idempotency key)
/// - `translated_at: <rfc3339>`
///
/// Structure preservation is delegated to [`crate::translate`]; the backend only
/// ever sees masked prose.
pub async fn build_twin_entry(
    backend: &dyn TranslateBackend,
    source: &co::entry::Entry,
    to_lang: &str,
) -> anyhow::Result<TwinOutcome> {
    let from_lang = source_lang(&source.frontmatter);
    let to_lang = to_lang.to_string();

    let translated = translate::translate_body(backend, &source.body, &from_lang, &to_lang).await?;
    let mut fm =
        translate::translate_frontmatter_title(backend, &source.frontmatter, &from_lang, &to_lang)
            .await?;

    if let Some(obj) = fm.as_object_mut() {
        obj.insert("lang".into(), serde_json::Value::String(to_lang.clone()));
        obj.insert(
            "translated_from".into(),
            serde_json::Value::String(source.path.clone()),
        );
        obj.insert(
            "translated_from_hash".into(),
            serde_json::Value::String(source.body_hash.clone()),
        );
        obj.insert(
            "translated_at".into(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
    }

    let path = twin_path(&source.path, &to_lang);
    let entry = crate::entry_index::make_entry(&path, fm, &translated.body);
    Ok(TwinOutcome {
        entry,
        from_lang,
        to_lang,
    })
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    // The entry path is a catch-all (`{*path}`), which Axum only permits at the
    // end of a route. So the action verb precedes the path:
    //   POST /{slug}/translate/{*path}?to=en
    //   GET  /{slug}/translation/{*path}
    Router::new()
        .route("/{slug}/translate/{*path}", post(translate_entry))
        .route("/{slug}/translation/{*path}", get(get_translation_info))
}

/// Require the caller owns `slug`. Mirrors `review_routes::require_owner`.
fn require_owner(state: &AppState, headers: &HeaderMap, slug: &str) -> Result<String, AppError> {
    let user_id = crate::auth::resolve_user_id(state, headers)
        .ok_or_else(|| AppError::Unauthorized("Login required".into()))?;
    let universe = state
        .core
        .storage_trait
        .get_universe(slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
    if universe.owner_id != user_id {
        return Err(AppError::Forbidden(
            "Not authorized: you do not own this universe".into(),
        ));
    }
    Ok(user_id)
}

/// POST .../entries/{path}/translate?to=en
pub async fn translate_entry(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    Query(q): Query<TranslateQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    // Owner-gated: only the universe owner can mint translations.
    require_owner(&state, &headers, &slug)?;

    let source = state
        .core
        .storage_trait
        .get_entry(&slug, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Entry '{path}' not found")))?;

    let from_lang = source_lang(&source.frontmatter);
    let to_lang = match q.to {
        Some(l) if translate::is_supported_lang(&l) => l,
        Some(l) => {
            return Err(AppError::BadRequest(format!(
                "Unsupported target language '{l}' (supported: pt, en)"
            )));
        }
        None => translate::opposite_lang(&from_lang).to_string(),
    };
    if to_lang == from_lang {
        return Err(AppError::BadRequest(format!(
            "Source entry is already in '{to_lang}'"
        )));
    }

    let twin = twin_path(&path, &to_lang);

    // Idempotency: if a twin already exists with translated_from_hash matching
    // the current source body, this is a no-op.
    if let Some(existing) = state
        .core
        .storage_trait
        .get_entry(&slug, &twin)
        .map_err(|e| AppError::Internal(e.to_string()))?
    {
        let prior_hash = existing
            .frontmatter
            .get("translated_from_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if prior_hash == source.body_hash {
            return Ok((
                StatusCode::OK,
                Json(TranslateResponse {
                    path: twin,
                    source_path: path,
                    lang: to_lang,
                    status: "unchanged".into(),
                    backend: "skipped".into(),
                }),
            )
                .into_response());
        }
    }

    // Build the configured backend. Unconfigured ⇒ 503 (graceful).
    let backend = translate::backend_from_env(std::sync::Arc::clone(&state.core.ai_router));
    if !backend.is_configured() {
        return Err(AppError::ServiceUnavailable(
            "Translation backend not configured. Set CO_TRANSLATE_BACKEND=llm to enable.".into(),
        ));
    }

    // Reuse the same Entry shape, then re-point it through the from_lang we
    // resolved (so the twin records the real source language).
    let outcome = build_twin_entry(backend.as_ref(), &source_as_entry(&source), &to_lang)
        .await
        .map_err(|e| AppError::Internal(format!("translation failed: {e}")))?;

    let status = if state
        .core
        .storage_trait
        .get_entry(&slug, &twin)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .is_some()
    {
        "updated"
    } else {
        "created"
    };

    write_and_index(&state, &slug, &outcome.entry)?;

    // CO-380: observability.
    state.core.eda_bus.publish(crate::eda::Event::new(
        "entry.translated",
        Some(slug.clone()),
        crate::auth::resolve_user_id(&state, &headers),
        serde_json::json!({
            "source": path,
            "twin": twin,
            "from": outcome.from_lang,
            "to": outcome.to_lang,
        }),
        crate::eda::Visibility::UniverseMembers,
    ));

    Ok((
        StatusCode::CREATED,
        Json(TranslateResponse {
            path: twin,
            source_path: path,
            lang: to_lang,
            status: status.into(),
            backend: backend.name().into(),
        }),
    )
        .into_response())
}

/// GET .../entries/{path}/translation — does a translation exist?
pub async fn get_translation_info(
    State(state): State<AppState>,
    Path((slug, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<TranslationInfo>, AppError> {
    let _ = &headers; // reads inherit the universe visibility gate (CO-161)

    let source = state
        .core
        .storage_trait
        .get_entry(&slug, &path)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Entry '{path}' not found")))?;

    let from_lang = source_lang(&source.frontmatter);
    let to_lang = translate::opposite_lang(&from_lang).to_string();
    let twin = twin_path(&path, &to_lang);

    match state
        .core
        .storage_trait
        .get_entry(&slug, &twin)
        .map_err(|e| AppError::Internal(e.to_string()))?
    {
        Some(existing) => {
            let prior_hash = existing
                .frontmatter
                .get("translated_from_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Ok(Json(TranslationInfo {
                exists: true,
                path: Some(twin),
                lang: Some(to_lang),
                stale: prior_hash != source.body_hash,
            }))
        }
        None => Ok(Json(TranslationInfo {
            exists: false,
            path: None,
            lang: None,
            stale: false,
        })),
    }
}

// ---------------------------------------------------------------------------
// Persistence helper (mirrors review_routes::write_and_index)
// ---------------------------------------------------------------------------

/// Convert the index `EntryRow` we read back into a core `Entry` for the
/// translation pipeline. We only need path/frontmatter/body/hash.
fn source_as_entry(row: &crate::entry_index::EntryRow) -> co::entry::Entry {
    crate::entry_index::make_entry(&row.path, row.frontmatter.clone(), &row.body)
}

/// Write an entry to disk + index it (file write, index upsert, semantic dates,
/// typed relations including `translated_from`). Never holds a lock across an
/// await — there are none here.
fn write_and_index(state: &AppState, slug: &str, entry: &co::entry::Entry) -> Result<(), AppError> {
    let universe_root = {
        let storage = state.core.storage.lock();
        storage.universe_root(slug)
    };
    co::write_entry(&universe_root, entry).map_err(|e| AppError::Internal(e.to_string()))?;

    let uc = {
        let storage = state.core.storage.lock();
        storage.universe_conn(slug)
    };
    let repo = crate::repository::SqliteEntryRepository::new(uc.clone());
    repo.upsert_entry(slug, entry)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    repo.upsert_dates(slug, entry, None)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    // CO-74: extract the typed `translated_from` relation + body wikilinks.
    let _ = crate::repository::SqliteRelationRepository::new(uc).sync_entry_relations(
        slug,
        &entry.path,
        &entry.entry_type,
        &entry.frontmatter,
        &entry.body,
        None,
    );

    let mut storage = state.core.storage.lock();
    storage.increment_universe_content_count(slug);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockBackend;

    #[async_trait]
    impl TranslateBackend for MockBackend {
        async fn translate(&self, text: &str, _f: &str, _t: &str) -> anyhow::Result<String> {
            assert!(!text.contains("```"), "backend saw a code fence");
            assert!(!text.contains("[["), "backend saw a wikilink");
            Ok(text.replace("Olá", "Hello").replace("mundo", "world"))
        }
        fn is_configured(&self) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "mock"
        }
    }

    fn source_entry() -> co::entry::Entry {
        let fm = serde_json::json!({
            "type": "nota",
            "lang": "pt",
            "title": "Olá mundo",
        });
        let body = "Olá mundo. Veja `co run`.\n\n```rust\nfn x() { /* Olá mundo */ }\n```\n\nVer [[topologia::x.md]].\n";
        crate::entry_index::make_entry("notas/saudacao.md", fm, body)
    }

    #[test]
    fn twin_path_appends_lang_before_md() {
        assert_eq!(twin_path("notas/x.md", "en"), "notas/x.en.md");
        assert_eq!(twin_path("a.md", "pt"), "a.pt.md");
    }

    #[test]
    fn twin_path_does_not_stack_lang_suffixes() {
        // Translating an en twin back to pt yields the pt twin, not a.en.pt.md.
        assert_eq!(twin_path("notas/x.en.md", "pt"), "notas/x.pt.md");
        assert_eq!(strip_lang_suffix("notas/x.en.md"), "notas/x.md");
        assert_eq!(strip_lang_suffix("notas/x.md"), "notas/x.md");
    }

    #[tokio::test]
    async fn build_twin_preserves_structure_and_stamps_provenance() {
        let src = source_entry();
        let twin = build_twin_entry(&MockBackend, &src, "en").await.unwrap();
        let e = twin.entry;

        // Path is the en sibling.
        assert_eq!(e.path, "notas/saudacao.en.md");

        // Structure preserved.
        assert!(e.body.contains("```rust"), "code fence lost: {}", e.body);
        assert!(
            e.body.contains("fn x() { /* Olá mundo */ }"),
            "code body translated (must not be): {}",
            e.body
        );
        assert!(e.body.contains("`co run`"), "inline code lost: {}", e.body);
        assert!(
            e.body.contains("[[topologia::x.md]]"),
            "wikilink lost: {}",
            e.body
        );
        assert!(e.body.contains("Hello world."), "prose not translated");

        // Provenance frontmatter.
        assert_eq!(e.frontmatter["lang"], "en");
        assert_eq!(e.frontmatter["translated_from"], "notas/saudacao.md");
        assert_eq!(e.frontmatter["translated_from_hash"], src.body_hash);
        assert_eq!(e.frontmatter["title"], "Hello world");
        assert_eq!(e.frontmatter["type"], "nota"); // machine field carried through
    }

    #[tokio::test]
    async fn build_twin_idempotency_key_is_source_body_hash() {
        // Re-running on the same source yields the same translated_from_hash,
        // which the route uses to short-circuit (unchanged). A changed body
        // yields a different hash.
        let src = source_entry();
        let twin_a = build_twin_entry(&MockBackend, &src, "en").await.unwrap();

        let mut changed = src.clone();
        changed.body = format!("{}\n\nNovo parágrafo.\n", src.body);
        changed.body_hash = co::entry::Entry::hash_body(&changed.body);
        let twin_b = build_twin_entry(&MockBackend, &changed, "en")
            .await
            .unwrap();

        assert_eq!(
            twin_a.entry.frontmatter["translated_from_hash"],
            src.body_hash
        );
        assert_ne!(
            twin_a.entry.frontmatter["translated_from_hash"],
            twin_b.entry.frontmatter["translated_from_hash"],
            "different source body must produce a different idempotency key"
        );
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::models::CreateUniverse;
    use crate::server::{
        AppState, AppStateInner, CoreState, IndexState, IntegrationsState, RealtimeState,
        build_router,
    };
    use crate::storage::Storage;

    const OWNER: &str = "owner-user";
    const SLUG: &str = "tr-test";

    fn test_config(dir: &std::path::Path) -> WebConfig {
        WebConfig {
            port: 0,
            data_dir: dir.to_str().unwrap().to_string(),
            static_dir: "co-web/static".to_string(),
            default_variant: "a".to_string(),
            experiments: false,
            plugins_dir: "plugins".to_string(),
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
        }
    }

    fn bearer_for(dir: &std::path::Path, user_id: &str) -> String {
        let storage = Storage::new(dir.to_str().unwrap());
        let token = storage
            .create_api_token(user_id, "translate-test")
            .unwrap()
            .token
            .unwrap();
        format!("Bearer {token}")
    }

    /// Seed a PUBLIC universe owned by OWNER with one pt source entry.
    fn seed(dir: &std::path::Path, state: &AppState) {
        {
            let mut storage = Storage::new(dir.to_str().unwrap());
            let _ = storage.create_universe(
                CreateUniverse {
                    key: SLUG.to_string(),
                    name: SLUG.to_string(),
                    description: String::new(),
                },
                OWNER,
            );
            storage
                .conn()
                .execute(
                    "UPDATE universes SET is_public = 1, visibility = 'public' WHERE key = ?1",
                    rusqlite::params![SLUG],
                )
                .unwrap();
        }
        let fm = serde_json::json!({ "type": "nota", "lang": "pt", "title": "Olá mundo" });
        let body = "Olá mundo. `co run`.\n\n```rust\nfn x() {}\n```\n\n[[topologia::x.md]]\n";
        let entry = crate::entry_index::make_entry("notas/saudacao.md", fm, body);
        write_and_index(state, SLUG, &entry).unwrap();
    }

    fn build_test_state(dir: &std::path::Path) -> AppState {
        let config = test_config(dir);
        let storage = Storage::new(&config.data_dir);
        let experiment = ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("game storage"));
        let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
        AppState::new(AppStateInner {
            core: Arc::new(CoreState::from_storage(storage, config, auth_store)),
            realtime: Arc::new(RealtimeState {
                doc_rooms: crate::ws::new_room_manager(),
                sync_rooms: crate::sync_ws::new_sync_room_manager(),
                chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
                chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
            index: Arc::new(IndexState {
                cache: crate::cache::CacheLayer::new(),
                embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
                embedding_tx,
            }),
            integrations: Arc::new(IntegrationsState {
                mail,
                geo: std::sync::Arc::new(crate::geo::GeoDb::disabled()),
                plugin_registry: game_core::plugin::PluginRegistry::new(),
                game_storage,
                wae: crate::wae::WaeEmitter::new(None, None),
                jwt_key: std::sync::Arc::new(crate::auth::JwtKey::load_or_generate()),
                rate_limiter: std::sync::Mutex::new(crate::rate_limit::InProcessRateLimiter::new()),
                experiment: Mutex::new(experiment),
                worker_supervisor: crate::infra::workers::InProcessExecutor::new_arc(),
            }),
        })
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    #[tokio::test]
    async fn anon_translate_is_unauthorized() {
        // CO-477: hold the env lock across remove → request so the backend-config
        // read during the request sees a stable (absent) CO_TRANSLATE_BACKEND.
        let _env = crate::test_support::env_lock().await;
        // Ensure no backend is configured for this test (default path).
        unsafe {
            std::env::remove_var("CO_TRANSLATE_BACKEND");
        }
        let dir = tempdir().unwrap();
        let state = build_test_state(dir.path());
        seed(dir.path(), &state);
        let app = build_router(state, None);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/universes/{SLUG}/translate/notas/saudacao.md?to=en"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn owner_translate_unconfigured_backend_returns_503() {
        // CO-477: hold the env lock across remove → request (see sibling test).
        let _env = crate::test_support::env_lock().await;
        unsafe {
            std::env::remove_var("CO_TRANSLATE_BACKEND");
        }
        let dir = tempdir().unwrap();
        let state = build_test_state(dir.path());
        seed(dir.path(), &state);
        let bearer = bearer_for(dir.path(), OWNER);
        let app = build_router(state, None);

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/v1/universes/{SLUG}/translate/notas/saudacao.md?to=en"
                    ))
                    .header(header::AUTHORIZATION, bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "no backend configured ⇒ graceful 503"
        );
    }

    #[tokio::test]
    async fn translation_info_reports_no_twin_then_existing() {
        let dir = tempdir().unwrap();
        let state = build_test_state(dir.path());
        seed(dir.path(), &state);
        let app = build_router(state.clone(), None);

        // No twin yet.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "/api/v1/universes/{SLUG}/translation/notas/saudacao.md"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let info = body_json(resp.into_body()).await;
        assert_eq!(info["exists"], false);

        // Seed a twin directly (simulating a prior translation) and re-query.
        let twin_fm = serde_json::json!({
            "type": "nota",
            "lang": "en",
            "translated_from": "notas/saudacao.md",
            "translated_from_hash": "deadbeef",
        });
        let twin =
            crate::entry_index::make_entry("notas/saudacao.en.md", twin_fm, "Hello world.\n");
        write_and_index(&state, SLUG, &twin).unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!(
                        "/api/v1/universes/{SLUG}/translation/notas/saudacao.md"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let info = body_json(resp.into_body()).await;
        assert_eq!(info["exists"], true);
        assert_eq!(info["path"], "notas/saudacao.en.md");
        assert_eq!(info["lang"], "en");
        // hash "deadbeef" != real source hash ⇒ stale.
        assert_eq!(info["stale"], true);
    }
}
