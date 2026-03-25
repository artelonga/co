use axum::Router;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Plugin manifest parsed from a `plugin.toml` file.
///
/// Every plugin ships a `plugin.toml` at its root with metadata and
/// universe configuration (map size, rules, theme, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique plugin identifier (e.g., "tetris").
    pub name: String,
    /// Semantic version (e.g., "0.1.0").
    pub version: String,
    /// Short description of the plugin.
    pub description: String,
    /// Plugin author.
    pub author: String,
    /// Universe-specific configuration.
    pub universe_config: UniverseConfig,
}

/// Configuration for a plugin's universe: map dimensions, tile data, rules,
/// theme colors, portals, and entities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseConfig {
    /// Map width in tiles.
    pub map_width: u32,
    /// Map height in tiles.
    pub map_height: u32,
    /// Tile data stored as a JSON value (flexible per-game schema).
    pub tile_data: serde_json::Value,
    /// Game rules as key-value pairs.
    pub rules: HashMap<String, serde_json::Value>,
    /// Theme color definitions (e.g., "background" -> "#1a1a2e").
    pub theme: HashMap<String, String>,
    /// Named portals connecting map locations.
    pub portals: Vec<PortalConfig>,
    /// Initial entity placements.
    pub entities: Vec<EntityConfig>,
}

/// A portal definition within the universe config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortalConfig {
    pub name: String,
    pub x: u32,
    pub y: u32,
    pub target_universe: Option<String>,
    pub target_x: Option<u32>,
    pub target_y: Option<u32>,
}

/// An entity placement within the universe config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityConfig {
    pub kind: String,
    pub x: u32,
    pub y: u32,
    pub properties: HashMap<String, serde_json::Value>,
}

/// Trait that all game plugins must implement.
///
/// Each plugin represents a self-contained game universe that can be loaded
/// into the platform. Plugins provide metadata via a TOML manifest and
/// implement game-specific logic through this trait.
pub trait Plugin: Send + Sync {
    /// Human-readable display name.
    fn name(&self) -> &str;

    /// Semantic version of the plugin (e.g., "0.1.0").
    fn version(&self) -> &str;

    /// Return the parsed plugin manifest.
    fn manifest(&self) -> &PluginManifest;

    /// Return an Axum router with plugin-specific API routes.
    fn routes(&self) -> Router;

    /// Called when the plugin is loaded into the platform.
    fn on_load(&mut self) {}

    /// Called when the plugin is unloaded from the platform.
    fn on_unload(&mut self) {}
}

/// Registry that manages loaded plugins.
pub struct PluginRegistry {
    plugins: HashMap<String, Box<dyn Plugin>>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Register a plugin. The plugin's manifest name is used as the key.
    /// Calls `on_load()` on the plugin after registration.
    pub fn register(&mut self, mut plugin: Box<dyn Plugin>) {
        let name = plugin.manifest().name.clone();
        plugin.on_load();
        self.plugins.insert(name, plugin);
    }

    /// Get a reference to a plugin by name.
    pub fn get(&self, name: &str) -> Option<&dyn Plugin> {
        self.plugins.get(name).map(|p| p.as_ref())
    }

    /// List all registered plugin names.
    pub fn list(&self) -> Vec<&str> {
        self.plugins.keys().map(|s| s.as_str()).collect()
    }

    /// Iterate over all registered plugins.
    pub fn iter(&self) -> impl Iterator<Item = &dyn Plugin> {
        self.plugins.values().map(|p| p.as_ref())
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockPlugin {
        manifest: PluginManifest,
        loaded: bool,
    }

    impl MockPlugin {
        fn new() -> Self {
            Self {
                manifest: PluginManifest {
                    name: "mock-game".to_string(),
                    version: "0.1.0".to_string(),
                    description: "A mock plugin for testing".to_string(),
                    author: "Test Author".to_string(),
                    universe_config: UniverseConfig {
                        map_width: 10,
                        map_height: 10,
                        tile_data: serde_json::json!([]),
                        rules: HashMap::new(),
                        theme: HashMap::from([("background".to_string(), "#000000".to_string())]),
                        portals: vec![],
                        entities: vec![],
                    },
                },
                loaded: false,
            }
        }
    }

    impl Plugin for MockPlugin {
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
            Router::new()
        }

        fn on_load(&mut self) {
            self.loaded = true;
        }
    }

    #[test]
    fn register_mock_plugin_and_verify_in_registry() {
        let mut registry = PluginRegistry::new();

        registry.register(Box::new(MockPlugin::new()));

        // Verify the plugin appears in the registry
        let names = registry.list();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"mock-game"));

        // Verify we can retrieve it by name
        let plugin = registry.get("mock-game").expect("plugin should exist");
        assert_eq!(plugin.name(), "mock-game");
        assert_eq!(plugin.version(), "0.1.0");
        assert_eq!(plugin.manifest().description, "A mock plugin for testing");
        assert_eq!(plugin.manifest().universe_config.map_width, 10);
    }
}
