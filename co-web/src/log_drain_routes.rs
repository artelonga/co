//! CO-124: Vercel Log Drain receiver — `POST /v1/log-drains/vercel/{universe_id}`.
//!
//! Validates the `x-vercel-signature` header (HMAC-SHA1 of raw request body using
//! the per-universe drain secret), maps Vercel's NDJSON log format to CO
//! TelemetryEvent records, and persists them to `log_drain_events` with
//! idempotent deduplication by `event_id`.
//!
//! Users configure a Vercel Log Drain pointing at:
//!   `POST https://co.artelonga.com.br/v1/log-drains/vercel/<universe-slug>`
//! and set the drain secret to the value returned by the universe settings API.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha1::Sha1;

use crate::server::AppState;
use crate::storage::LogDrainEvent;

type HmacSha1 = Hmac<Sha1>;

pub fn router() -> Router<AppState> {
    Router::new().route("/{universe_id}", post(vercel_drain_handler))
}

/// A single entry from Vercel's NDJSON Log Drain stream.
///
/// <https://vercel.com/docs/observability/log-drains-overview/log-drains-reference>
#[derive(Deserialize)]
struct VercelLogEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    path: String,
}

async fn vercel_drain_handler(
    Path(universe_id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    // 1. Look up the per-universe drain secret.
    let secret = {
        let storage = state.storage.lock();
        match storage.get_log_drain_secret(&universe_id) {
            Ok(Some(s)) if !s.is_empty() => s,
            Ok(_) => return StatusCode::NOT_FOUND,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
        }
    };

    // 2. Validate Vercel HMAC-SHA1 signature.
    let sig_header = headers
        .get("x-vercel-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_vercel_signature(secret.as_bytes(), &body, sig_header) {
        return StatusCode::UNAUTHORIZED;
    }

    // 3. Parse NDJSON.
    let events: Vec<VercelLogEntry> = body
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect();

    if events.is_empty() {
        return StatusCode::OK;
    }

    // 4. Persist events — deduplicated by event_id via INSERT OR IGNORE.
    let received_at = Utc::now().to_rfc3339();
    let storage = state.storage.lock();
    for event in &events {
        let event_id = if event.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            event.id.clone()
        };
        let _ = storage.insert_log_drain_event(&LogDrainEvent {
            event_id,
            universe_id: universe_id.clone(),
            received_at: received_at.clone(),
            source: event.source.clone(),
            level: event.level.clone(),
            message: event.message.clone(),
            host: event.host.clone(),
            path: event.path.clone(),
        });
    }

    StatusCode::OK
}

/// Validate the `x-vercel-signature` header value.
///
/// Vercel computes `HMAC-SHA1(key=secret, data=body)` and sends the result as
/// `sha1=<hex>`.  Returns `false` on any format or crypto error.
pub fn verify_vercel_signature(secret: &[u8], body: &[u8], header: &str) -> bool {
    let expected_hex = match header.strip_prefix("sha1=") {
        Some(h) if !h.is_empty() => h,
        _ => return false,
    };
    let Ok(mut mac) = HmacSha1::new_from_slice(secret) else {
        return false;
    };
    mac.update(body);
    let result = mac.finalize().into_bytes();
    let computed: String = result.iter().map(|b| format!("{b:02x}")).collect();
    computed == expected_hex
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::config::WebConfig;
    use crate::experiment::ExperimentStore;
    use crate::models::CreateUniverse;
    use crate::server::{AppState, AppStateInner, build_router};
    use crate::storage::Storage;

    fn make_sig(secret: &[u8], body: &[u8]) -> String {
        let mut mac = HmacSha1::new_from_slice(secret).unwrap();
        mac.update(body);
        let result = mac.finalize().into_bytes();
        let hex: String = result.iter().map(|b| format!("{b:02x}")).collect();
        format!("sha1={hex}")
    }

    fn build_test_app(dir: &std::path::Path) -> axum::Router {
        let config = WebConfig {
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
        };
        let storage = Storage::new(&config.data_dir);
        let experiment = ExperimentStore::new(&config.data_dir);
        let auth_store = crate::auth::AuthStore::new(dir).unwrap();
        let mail: Arc<dyn co::MailProvider> = Arc::new(co::LogMailProvider);
        let game_db_path = dir.join("game_test.db");
        let game_storage =
            Arc::new(game_core::storage::Storage::open(&game_db_path).expect("open game storage"));
        let (embedding_tx, _embedding_rx) = crate::embedding_worker::channel();
        let state: AppState = Arc::new(AppStateInner {
            storage: parking_lot::Mutex::new(storage),
            experiment: Mutex::new(experiment),
            config,
            auth_store: Mutex::new(auth_store),
            mail,
            game_storage,
            plugin_registry: game_core::plugin::PluginRegistry::new(),
            doc_rooms: crate::ws::new_room_manager(),
            sync_rooms: crate::sync_ws::new_sync_room_manager(),
            cache: crate::cache::CacheLayer::new(),
            rate_limiter: Mutex::new(crate::rate_limit::RateLimiter::new()),
            wae: crate::wae::WaeEmitter::new(None, None),
            jwt_key: Arc::new(crate::auth::JwtKey::load_or_generate()),
            embeddings: std::sync::Arc::new(crate::embedding::EmbeddingService::disabled()),
            embedding_tx,
            chat_rooms_broadcast: std::sync::Mutex::new(std::collections::HashMap::new()),
            chat_presence: std::sync::Mutex::new(std::collections::HashMap::new()),
        });
        build_router(state, None)
    }

    fn seed_universe_with_secret(dir: &std::path::Path, universe_id: &str, secret: &str) {
        let mut storage = Storage::new(dir);
        let _ = storage.create_universe(
            CreateUniverse {
                key: universe_id.to_string(),
                name: universe_id.to_string(),
                description: String::new(),
            },
            "test-owner",
        );
        storage
            .set_log_drain_secret(universe_id, secret)
            .expect("set drain secret");
    }

    // ── T1: valid signature + valid payload → 200 ────────────────────────────
    #[tokio::test]
    async fn test_valid_drain_payload_accepted() {
        let dir = tempdir().unwrap();
        let secret = b"test-drain-secret";
        seed_universe_with_secret(dir.path(), "drain-test", "test-drain-secret");

        let body = r#"{"id":"evt-001","message":"hello world","source":"lambda","level":"info","host":"app.vercel.app","path":"/api/test"}"#;
        let sig = make_sig(secret, body.as_bytes());

        let resp = build_test_app(dir.path())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/log-drains/vercel/drain-test")
                    .header("x-vercel-signature", sig)
                    .header("content-type", "application/x-ndjson")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── T2: invalid signature → 401 ──────────────────────────────────────────
    #[tokio::test]
    async fn test_invalid_signature_rejected() {
        let dir = tempdir().unwrap();
        seed_universe_with_secret(dir.path(), "sig-test", "correct-secret");

        let resp = build_test_app(dir.path())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/log-drains/vercel/sig-test")
                    .header("x-vercel-signature", "sha1=deadbeef")
                    .header("content-type", "application/x-ndjson")
                    .body(Body::from(r#"{"id":"evt-002","message":"x"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── T3: unknown universe → 404 ───────────────────────────────────────────
    #[tokio::test]
    async fn test_unknown_universe_returns_404() {
        let dir = tempdir().unwrap();

        let resp = build_test_app(dir.path())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/log-drains/vercel/no-such-universe")
                    .header("x-vercel-signature", "sha1=anything")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── T4: duplicate event_id stored only once ───────────────────────────────
    #[tokio::test]
    async fn test_duplicate_event_deduplicated() {
        let dir = tempdir().unwrap();
        let secret = b"dedup-secret";
        seed_universe_with_secret(dir.path(), "dedup-u", "dedup-secret");

        let body = r#"{"id":"evt-dedup","message":"once","source":"lambda","level":"info","host":"x.vercel.app","path":"/"}"#;
        let sig = make_sig(secret, body.as_bytes());

        for _ in 0..2 {
            let resp = build_test_app(dir.path())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/v1/log-drains/vercel/dedup-u")
                        .header("x-vercel-signature", &sig)
                        .header("content-type", "application/x-ndjson")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }

        let storage = Storage::new(dir.path());
        let count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM log_drain_events WHERE event_id = 'evt-dedup'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "duplicate event must be stored exactly once");
    }

    // ── T5: pure signature verification ──────────────────────────────────────
    #[test]
    fn test_verify_vercel_signature_pure() {
        let secret = b"my-secret";
        let body = b"hello vercel";
        let sig = make_sig(secret, body);
        assert!(verify_vercel_signature(secret, body, &sig));
        assert!(!verify_vercel_signature(secret, body, "sha1=badhex"));
        assert!(!verify_vercel_signature(secret, body, "no-prefix"));
        assert!(!verify_vercel_signature(b"wrong-key", body, &sig));
    }
}
