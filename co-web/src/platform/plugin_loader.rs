use axum::{Json, Router, routing::get};
use game_core::plugin::{Plugin, PluginManifest, PluginRegistry, RouteDescriptor};
use game_core::storage::Storage;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// A plugin loaded from a `plugin.toml` manifest on disk.
struct ManifestPlugin {
    manifest: PluginManifest,
}

impl Plugin for ManifestPlugin {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn version(&self) -> &str {
        &self.manifest.version
    }

    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn routes(&self) -> Vec<RouteDescriptor> {
        // CO-436: game-core is now axum-free, so plugins describe their routes
        // portably. The host (this module) translates each descriptor into a
        // concrete axum route — see `descriptors_to_router`.
        vec![RouteDescriptor {
            path: "/info".to_string(),
            method: "GET".to_string(),
            handler_id: "info".to_string(),
        }]
    }
}

/// Translate a plugin's framework-agnostic [`RouteDescriptor`]s into an axum
/// [`Router`], wiring each `handler_id` to its concrete handler.
///
/// CO-436: this is the co-web side of the portable-plugin boundary. `game-core`
/// no longer returns an `axum::Router`; the host owns the HTTP translation.
/// Unknown handler ids are logged and skipped so a forward-compatible plugin
/// can't crash the loader.
fn descriptors_to_router(manifest: &PluginManifest, descriptors: &[RouteDescriptor]) -> Router {
    let mut router = Router::new();
    for desc in descriptors {
        match (desc.method.as_str(), desc.handler_id.as_str()) {
            ("GET", "info") => {
                let manifest = manifest.clone();
                router = router.route(
                    &desc.path,
                    get(move || {
                        let m = manifest.clone();
                        async move {
                            Json(serde_json::json!({
                                "name": m.name,
                                "version": m.version,
                                "description": m.description
                            }))
                        }
                    }),
                );
            }
            _ => {
                warn!(
                    method = %desc.method,
                    handler_id = %desc.handler_id,
                    path = %desc.path,
                    "unknown plugin route descriptor, skipping"
                );
            }
        }
    }
    router
}

/// Scan `plugins_dir` for subdirectories containing `plugin.toml`,
/// parse each manifest, register plugins, store universe configs in
/// the database on first load, and return the populated registry plus
/// a merged router for all plugin routes.
pub fn load_plugins(plugins_dir: &Path, storage: &Storage) -> (PluginRegistry, Router) {
    let mut registry = PluginRegistry::new();
    let mut plugin_router = Router::new();

    if !plugins_dir.is_dir() {
        warn!(
            path = %plugins_dir.display(),
            "plugins directory does not exist, no plugins loaded"
        );
        return (registry, plugin_router);
    }

    let entries = match std::fs::read_dir(plugins_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(
                path = %plugins_dir.display(),
                error = %e,
                "failed to read plugins directory"
            );
            return (registry, plugin_router);
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "failed to read directory entry");
                continue;
            }
        };

        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let toml_path = path.join("plugin.toml");
        if !toml_path.exists() {
            warn!(
                dir = %path.display(),
                "directory in plugins/ has no plugin.toml, skipping"
            );
            continue;
        }

        match load_single_plugin(&toml_path, storage) {
            Ok(plugin) => {
                let name = plugin.manifest.name.clone();
                let version = plugin.manifest.version.clone();
                let descriptors = plugin.routes();
                let route_count = descriptors.len();
                let routes = descriptors_to_router(&plugin.manifest, &descriptors);
                let route_prefix = format!("/{}", name);

                plugin_router = plugin_router.nest(&route_prefix, routes);

                info!(
                    name = %name,
                    version = %version,
                    routes = route_count,
                    "loaded plugin"
                );

                registry.register(Box::new(plugin));
            }
            Err(e) => {
                warn!(
                    path = %toml_path.display(),
                    error = %e,
                    "failed to load plugin, skipping"
                );
            }
        }
    }

    (registry, plugin_router)
}

/// Parse a single `plugin.toml` and store its universe config in the database
/// if it doesn't already exist.
fn load_single_plugin(
    toml_path: &Path,
    storage: &Storage,
) -> std::result::Result<ManifestPlugin, String> {
    let contents = std::fs::read_to_string(toml_path)
        .map_err(|e| format!("failed to read {}: {}", toml_path.display(), e))?;

    let manifest: PluginManifest = toml::from_str(&contents)
        .map_err(|e| format!("malformed plugin.toml at {}: {}", toml_path.display(), e))?;

    // Store universe config in database on first load
    match storage.has_plugin_config(&manifest.name) {
        Ok(false) => {
            let config_json = serde_json::to_string(&manifest.universe_config)
                .map_err(|e| format!("failed to serialize universe config: {}", e))?;
            storage
                .save_plugin_config(&manifest.name, &config_json)
                .map_err(|e| format!("failed to store plugin config in database: {}", e))?;
            info!(
                name = %manifest.name,
                "stored universe config in database (first load)"
            );
        }
        Ok(true) => {
            info!(
                name = %manifest.name,
                "universe config already in database, skipping store"
            );
        }
        Err(e) => {
            warn!(
                name = %manifest.name,
                error = %e,
                "failed to check plugin config in database, storing anyway"
            );
            let config_json = serde_json::to_string(&manifest.universe_config)
                .map_err(|e| format!("failed to serialize universe config: {}", e))?;
            let _ = storage.save_plugin_config(&manifest.name, &config_json);
        }
    }

    Ok(ManifestPlugin { manifest })
}

/// Returns the configured plugins directory path.
pub fn plugins_dir() -> PathBuf {
    PathBuf::from(crate::infra::secrets::global().get_or("PLUGINS_DIR", "plugins"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const VALID_PLUGIN_TOML: &str = r##"
name = "test-game"
version = "0.1.0"
description = "A test plugin"
author = "Test Author"

[universe_config]
map_width = 20
map_height = 15
tile_data = []
portals = []
entities = []

[universe_config.rules]

[universe_config.theme]
background = "#1a1a2e"
"##;

    fn test_storage() -> (Arc<Storage>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let storage = Arc::new(Storage::open(&db_path).unwrap());
        (storage, tmp)
    }

    #[test]
    fn load_valid_plugin_from_directory() {
        let (storage, _tmp) = test_storage();
        let plugins_dir = tempfile::tempdir().unwrap();
        let plugin_dir = plugins_dir.path().join("test-game");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.toml"), VALID_PLUGIN_TOML).unwrap();

        let (registry, _router) = load_plugins(plugins_dir.path(), &storage);

        assert_eq!(registry.len(), 1);
        let names = registry.list();
        assert!(names.contains(&"test-game"));

        let plugin = registry.get("test-game").unwrap();
        assert_eq!(plugin.version(), "0.1.0");
        assert_eq!(plugin.manifest().description, "A test plugin");
    }

    #[test]
    fn malformed_toml_does_not_crash() {
        let (storage, _tmp) = test_storage();
        let plugins_dir = tempfile::tempdir().unwrap();
        let plugin_dir = plugins_dir.path().join("bad-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.toml"), "this is not valid toml {{{").unwrap();

        let (registry, _router) = load_plugins(plugins_dir.path(), &storage);

        assert!(registry.is_empty());
    }

    #[test]
    fn missing_plugins_dir_does_not_crash() {
        let (storage, _tmp) = test_storage();
        let nonexistent = PathBuf::from("/tmp/nonexistent_plugins_dir_abc123");

        let (registry, _router) = load_plugins(&nonexistent, &storage);

        assert!(registry.is_empty());
    }

    #[test]
    fn universe_config_stored_in_database_on_first_load() {
        let (storage, _tmp) = test_storage();
        let plugins_dir = tempfile::tempdir().unwrap();
        let plugin_dir = plugins_dir.path().join("test-game");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.toml"), VALID_PLUGIN_TOML).unwrap();

        // First load should store config
        let (registry, _) = load_plugins(plugins_dir.path(), &storage);
        assert_eq!(registry.len(), 1);

        let config = storage.get_plugin_config("test-game").unwrap();
        assert!(config.is_some());

        let config_json: serde_json::Value = serde_json::from_str(&config.unwrap()).unwrap();
        assert_eq!(config_json["map_width"], 20);
        assert_eq!(config_json["map_height"], 15);
    }

    #[test]
    fn directory_without_toml_is_skipped() {
        let (storage, _tmp) = test_storage();
        let plugins_dir = tempfile::tempdir().unwrap();
        let empty_dir = plugins_dir.path().join("no-manifest");
        std::fs::create_dir_all(&empty_dir).unwrap();

        let (registry, _router) = load_plugins(plugins_dir.path(), &storage);

        assert!(registry.is_empty());
    }

    #[test]
    fn multiple_plugins_loaded() {
        let (storage, _tmp) = test_storage();
        let plugins_dir = tempfile::tempdir().unwrap();

        for name in &["alpha", "beta"] {
            let dir = plugins_dir.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let toml = format!(
                r#"
name = "{name}"
version = "1.0.0"
description = "{name} plugin"
author = "Author"

[universe_config]
map_width = 10
map_height = 10
tile_data = []
portals = []
entities = []

[universe_config.rules]

[universe_config.theme]
"#
            );
            std::fs::write(dir.join("plugin.toml"), toml).unwrap();
        }

        let (registry, _router) = load_plugins(plugins_dir.path(), &storage);

        assert_eq!(registry.len(), 2);
        assert!(registry.get("alpha").is_some());
        assert!(registry.get("beta").is_some());
    }

    /// CO-436: game-core now hands back framework-agnostic `RouteDescriptor`s;
    /// the host translates them into a working axum route. This locks the
    /// translation: the descriptor `GET /info` → a route that serves the
    /// manifest JSON, identical to the pre-refactor behavior.
    #[tokio::test]
    async fn descriptor_info_route_serves_manifest_json() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let manifest: PluginManifest = toml::from_str(VALID_PLUGIN_TOML).unwrap();
        let descriptors = ManifestPlugin {
            manifest: manifest.clone(),
        }
        .routes();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].handler_id, "info");

        let router = descriptors_to_router(&manifest, &descriptors);
        let resp = router
            .oneshot(Request::builder().uri("/info").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["name"], "test-game");
        assert_eq!(json["version"], "0.1.0");
        assert_eq!(json["description"], "A test plugin");
    }

    /// Unknown handler ids are skipped (forward-compatibility), not panicked on.
    #[test]
    fn unknown_descriptor_is_skipped() {
        let manifest: PluginManifest = toml::from_str(VALID_PLUGIN_TOML).unwrap();
        let descriptors = vec![RouteDescriptor {
            path: "/future".to_string(),
            method: "POST".to_string(),
            handler_id: "not-yet-implemented".to_string(),
        }];
        // Should build without panicking even though the handler is unknown.
        let _router = descriptors_to_router(&manifest, &descriptors);
    }
}
