//! REPL shell over the entries API.
//!
//! Serves a single static page at `/repl`. The page tokenizes user
//! input client-side, dispatches commands (`list`, `get`, `put`,
//! `delete`, `save`) to `/api/v1/universes/.../entries/...`, and can
//! save the transcript as a markdown entry under
//! `notebooks/<name>.md` in the active universe.
//!
//! Roadmap (see CHANGELOG 2.7.18):
//!   - Lazy Python kernel via Pyodide (next)
//!   - R kernel via WebR (follow-up)
//!   - `x-runnable` markdown fences for "include as code"

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::Response,
};

use crate::server::CoreState;

const REPL_HTML: &str = include_str!("../../static/shared/repl.html");

pub async fn serve_repl_page(State(_core): State<Arc<CoreState>>) -> Response {
    let mut resp = Response::new(REPL_HTML.into());
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    *resp.status_mut() = StatusCode::OK;
    resp
}
