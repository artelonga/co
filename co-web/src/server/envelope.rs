//! CO-456 (CO-278-A): opt-in API response envelope `{ data, meta, errors }`
//! plus always-on version headers, implemented as a **single response layer**
//! over every `/api/v1/*` route — no per-handler edits.
//!
//! Design constraints (owner decision 2026-06-14):
//! - The SPA has 55 raw `fetch()` call sites; an *unconditional* envelope would
//!   break it and every external consumer on deploy. So the envelope is
//!   **opt-in** via the `X-API-Envelope: 1` request header (or
//!   `Accept: application/vnd.co.v1+json`). Without the marker the response body
//!   is passed through **byte-identical**.
//! - Non-JSON responses (HTML, CSS, streams, blobs) are **never** wrapped.
//! - The version headers (`X-API-Version`, `X-Co-Server-Version`) are additive —
//!   they don't touch the body — so they ride on **every** `/api/v1/*` response,
//!   with or without opt-in.
//!
//! Flipping the envelope to the default is a later one-liner (drop the opt-in
//! gate) once consumers have adopted the header.

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, HeaderName, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use serde_json::{Value, json};

/// `X-API-Version` value advertised on every `/api/v1/*` response.
const API_VERSION: &str = "1.0";

/// Opt-in request header. `X-API-Envelope: 1` turns the envelope on.
const ENVELOPE_HEADER: &str = "x-api-envelope";

/// Opt-in media type (alternative to the header).
const ENVELOPE_ACCEPT: &str = "application/vnd.co.v1+json";

/// Upper bound on a body we will buffer to re-wrap (1 MiB — matches the request
/// `DefaultBodyLimit`). Anything larger is passed through unwrapped.
const MAX_WRAP_BYTES: usize = 1_048_576;

/// Single response layer for the public API. Gated on the `/api/v1/` path prefix
/// so it never touches SPA shells, static files, or non-versioned endpoints.
pub async fn envelope_middleware(req: Request<Body>, next: Next) -> Response {
    // Only the versioned public API surface participates.
    if !req.uri().path().starts_with("/api/v1") {
        return next.run(req).await;
    }

    let opt_in = wants_envelope(req.headers());
    // The outer `SetRequestIdLayer` sets `x-request-id`; reuse it so `meta`
    // matches the propagated header. Fall back to a fresh uuid defensively.
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let mut resp = next.run(req).await;

    if opt_in && is_json(resp.headers()) {
        resp = wrap_response(resp, &request_id).await;
    }

    // Version headers are additive and always present on /api/v1/* responses.
    add_version_headers(resp.headers_mut());
    resp
}

/// True when the caller opted into the envelope via header or media type.
fn wants_envelope(headers: &HeaderMap) -> bool {
    if let Some(v) = headers.get(ENVELOPE_HEADER).and_then(|v| v.to_str().ok())
        && v.trim() == "1"
    {
        return true;
    }
    headers
        .get(ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|a| a.contains(ENVELOPE_ACCEPT))
        .unwrap_or(false)
}

/// True when the response advertises a JSON content type.
fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.starts_with("application/json"))
        .unwrap_or(false)
}

fn add_version_headers(headers: &mut HeaderMap) {
    headers.insert(
        HeaderName::from_static("x-api-version"),
        HeaderValue::from_static(API_VERSION),
    );
    headers.insert(
        HeaderName::from_static("x-co-server-version"),
        HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
    );
}

/// Buffer a JSON response and re-emit it inside the envelope. Success bodies
/// (`< 400`) go under `data`; error bodies become an `errors` array.
async fn wrap_response(resp: Response, request_id: &str) -> Response {
    let status = resp.status();
    let (mut parts, body) = resp.into_parts();

    let bytes = match axum::body::to_bytes(body, MAX_WRAP_BYTES).await {
        Ok(b) => b,
        // Body too large or unreadable: it is already consumed, so we cannot
        // pass the original through. Emit an enveloped 500 instead.
        Err(_) => {
            let env = json!({
                "data": Value::Null,
                "meta": meta(request_id, None),
                "errors": [{
                    "code": "envelope_buffer_failed",
                    "message": "Response body too large to envelope",
                }],
            });
            return json_response(StatusCode::INTERNAL_SERVER_ERROR, &mut parts.headers, &env);
        }
    };

    // Content-type claimed JSON but the body isn't valid JSON: pass it through
    // untouched rather than corrupt it.
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };

    let envelope = if status.as_u16() < 400 {
        json!({
            "data": value,
            "meta": meta(request_id, Some(&value)),
            "errors": Value::Null,
        })
    } else {
        json!({
            "data": Value::Null,
            "meta": meta(request_id, None),
            "errors": [error_object(&value, status)],
        })
    };

    json_response(status, &mut parts.headers, &envelope)
}

/// Build the `meta` object: always `request_id` + `api_version`, plus
/// best-effort pagination (`page`/`page_size`/`total`) when the success body
/// already exposes those fields. Never invents pagination.
fn meta(request_id: &str, body: Option<&Value>) -> Value {
    let mut m = json!({
        "request_id": request_id,
        "api_version": API_VERSION,
    });
    if let Some(Value::Object(obj)) = body {
        let map = m.as_object_mut().expect("meta is an object");
        for key in ["page", "page_size", "total"] {
            if let Some(v) = obj.get(key) {
                map.insert(key.to_string(), v.clone());
            }
        }
    }
    m
}

/// Map an existing JSON error body to an `{ code, message, field?, hint? }`
/// entry. Best-effort: handlers emit a few shapes (`{error, message, message_en}`
/// from `AppError`, `{error}` from simple handlers), so we read what's there and
/// fall back to the HTTP status reason.
fn error_object(value: &Value, status: StatusCode) -> Value {
    let reason = status.canonical_reason().unwrap_or("Error");
    let obj = value.as_object();

    let str_field = |key: &str| {
        obj.and_then(|o| o.get(key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    let code = str_field("error").unwrap_or_else(|| reason.to_string());
    let message = str_field("message_en")
        .or_else(|| str_field("message"))
        .or_else(|| str_field("error"))
        .unwrap_or_else(|| reason.to_string());

    let mut entry = json!({ "code": code, "message": message });
    let map = entry.as_object_mut().expect("error entry is an object");
    if let Some(field) = str_field("field") {
        map.insert("field".to_string(), Value::String(field));
    }
    if let Some(hint) = str_field("hint") {
        map.insert("hint".to_string(), Value::String(hint));
    }
    entry
}

/// Re-serialize `body` as the response, preserving the original headers (minus a
/// now-stale `content-length`) and forcing a JSON content type.
fn json_response(status: StatusCode, headers: &mut HeaderMap, body: &Value) -> Response {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    headers.remove(CONTENT_LENGTH);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let mut resp = Response::new(Body::from(bytes));
    *resp.status_mut() = status;
    *resp.headers_mut() = std::mem::take(headers);
    resp
}
