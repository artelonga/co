//! Blob CAS API — content-addressed read/write over the meta-DB `blobs`
//! table seeded by Phase 8 step 1 (1.70.0). Foundation for external
//! tools (mempalace BaseBackend shim, future MCP server, agents that
//! need raw content by hash) to operate on CO's deduplicated content
//! corpus without going through the per-entry endpoints.
//!
//! Auth: all routes require authentication. Hashes are 256-bit and not
//! enumerable in practice, but gating on auth keeps the surface honest
//! — anonymous readers go through the visibility-gated entry routes.
//!
//! Endpoints:
//! - `GET /api/v1/blobs/:hash`  — bytes (`application/octet-stream`)
//! - `HEAD /api/v1/blobs/:hash` — 200 if exists, 404 otherwise
//! - `POST /api/v1/blobs`       — body bytes; returns `{hash, size}`

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Serialize;

use crate::auth::UserId;
use crate::error::AppError;
use crate::server::AppState;

#[derive(Debug, Serialize)]
pub struct PutBlobResponse {
    pub hash: String,
    pub size: usize,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/blobs/{hash}", get(get_blob).head(head_blob))
        .route("/blobs", post(post_blob))
}

/// `GET /api/v1/blobs/:hash` — return raw bytes.
pub async fn get_blob(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    _user: UserId,
) -> Result<impl IntoResponse, AppError> {
    if !is_plausible_hash(&hash) {
        return Err(AppError::BadRequest(
            "hash must be 64 hex chars (sha256)".into(),
        ));
    }
    let storage = state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("storage lock failed".into()))?;
    let bytes = storage
        .get_blob(&hash)
        .ok_or_else(|| AppError::NotFound(format!("blob '{hash}' not found")))?;
    Ok((
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
            (
                axum::http::header::CACHE_CONTROL,
                "public, max-age=31536000, immutable",
            ),
        ],
        bytes,
    ))
}

/// `HEAD /api/v1/blobs/:hash` — exists check, no body.
pub async fn head_blob(
    State(state): State<AppState>,
    Path(hash): Path<String>,
    _user: UserId,
) -> Result<StatusCode, AppError> {
    if !is_plausible_hash(&hash) {
        return Err(AppError::BadRequest(
            "hash must be 64 hex chars (sha256)".into(),
        ));
    }
    let storage = state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("storage lock failed".into()))?;
    if storage.has_blob(&hash) {
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

/// `POST /api/v1/blobs` — store bytes, return content hash.
/// Idempotent — same bytes always return the same hash.
pub async fn post_blob(
    State(state): State<AppState>,
    _user: UserId,
    body: Bytes,
) -> Result<Json<PutBlobResponse>, AppError> {
    let storage = state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("storage lock failed".into()))?;
    let hash = storage
        .put_blob(&body)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(PutBlobResponse {
        hash,
        size: body.len(),
    }))
}

fn is_plausible_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_plausible_hash() {
        // 64 hex chars
        let valid = "0".repeat(64);
        assert!(is_plausible_hash(&valid));
        // sha256 of "" — well-known
        assert!(is_plausible_hash(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        // wrong length
        assert!(!is_plausible_hash("abc"));
        assert!(!is_plausible_hash(&"0".repeat(63)));
        assert!(!is_plausible_hash(&"0".repeat(65)));
        // non-hex
        let bad: String = "z".repeat(64);
        assert!(!is_plausible_hash(&bad));
    }
}
