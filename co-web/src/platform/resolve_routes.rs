//! CO-338: surface-key resolution endpoint.
//!
//! `GET /api/v1/resolve?ref=<key>::<path>` turns a logical `key::path`
//! cross-universe reference into a live deployment URL by walking the universe
//! registry (`key`, `parent_key`, `surface_dns`) to the nearest deployable
//! ancestor. The resolution logic lives in [`co::surface`] so the CLI, co-web,
//! and standalone surfaces share one implementation.

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::server::AppState;

#[derive(Debug, Deserialize)]
pub struct ResolveQuery {
    /// The reference to resolve, e.g. `mbya::` or `yggdrasil::comunicacao/mbya`.
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Serialize)]
pub struct ResolveResponse {
    /// The live deployment URL.
    pub url: String,
    /// The resolved node's universe key.
    pub universe: String,
    /// The deployable ancestor whose DNS provided the base, or `null` when the
    /// platform default host was used.
    pub deployable_ancestor: Option<String>,
}

/// Build the surface registry from the universe rows (held lock kept short).
pub fn registry(state: &AppState) -> co::surface::SurfaceRegistry {
    let nodes = state.core.storage.lock().list_surface_nodes();
    co::surface::SurfaceRegistry::new(nodes)
}

/// GET /api/v1/resolve?ref=<key>::<path>
pub async fn resolve(
    State(state): State<AppState>,
    Query(q): Query<ResolveQuery>,
) -> Result<Json<ResolveResponse>, AppError> {
    let registry = registry(&state);
    let resolved = registry.resolve(&q.reference).map_err(|e| match e {
        co::surface::SurfaceError::UnknownKey(_) => AppError::NotFound(e.to_string()),
        co::surface::SurfaceError::Ambiguous { .. }
        | co::surface::SurfaceError::NotASurfaceRef(_) => AppError::BadRequest(e.to_string()),
    })?;
    Ok(Json(ResolveResponse {
        url: resolved.url,
        universe: resolved.universe,
        deployable_ancestor: resolved.deployable_ancestor,
    }))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/resolve", get(resolve))
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    /// Build the yggdrasil → comunicacao → mbya tree (yggdrasil deployable) and
    /// exercise the exact storage → registry → resolve path the handler uses.
    fn seeded_storage() -> Storage {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        storage.ensure_local_universe("yggdrasil", "Yggdrasil", true);
        storage.ensure_local_universe("comunicacao", "Comunicação", true);
        storage.ensure_local_universe("mbya", "Mbya", true);
        storage
            .conn()
            .execute(
                "UPDATE universes SET surface_dns = 'yggdrasil.artelonga.com.br' WHERE key = 'yggdrasil'",
                [],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "UPDATE universes SET parent_key = 'yggdrasil' WHERE key = 'comunicacao'",
                [],
            )
            .unwrap();
        storage
            .conn()
            .execute(
                "UPDATE universes SET parent_key = 'comunicacao' WHERE key = 'mbya'",
                [],
            )
            .unwrap();
        // Leak the tempdir so the SQLite files outlive the returned Storage.
        std::mem::forget(dir);
        storage
    }

    #[test]
    fn registry_from_storage_resolves_nested_ref() {
        let storage = seeded_storage();
        let nodes = storage.list_surface_nodes();
        let registry = co::surface::SurfaceRegistry::new(nodes);

        let r = registry.resolve("mbya::").expect("resolves");
        assert_eq!(r.url, "https://yggdrasil.artelonga.com.br/comunicacao/mbya");
        assert_eq!(r.universe, "mbya");
        assert_eq!(r.deployable_ancestor.as_deref(), Some("yggdrasil"));

        // Unknown key surfaces as an error the handler maps to 404.
        assert!(matches!(
            registry.resolve("nope::"),
            Err(co::surface::SurfaceError::UnknownKey(_))
        ));
    }
}
