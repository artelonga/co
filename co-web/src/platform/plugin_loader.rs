use axum::{Json, Router, routing::get};
use game_core::plugin::{Plugin, PluginManifest, PluginRegistry};
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

    fn routes(&self) -> Router {
        let manifest = self.manifest.clone();
        Router::new().route(
            "/info",
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
        )
    }
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
                let routes = plugin.routes();
                let route_prefix = format!("/{}", name);

                plugin_router = plugin_router.nest(&route_prefix, routes);

                info!(
                    name = %name,
                    version = %version,
                    routes = 1,
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
}
