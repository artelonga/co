//! Feature system for CO extensions (agents, tools, etc.)
//!
//! Features are extension interfaces with their own types, queries, and logic.
//! Each feature has:
//! - A directory at repo root (e.g., `agents/`, `tools/`)
//! - An optional schema.yaml defining property types
//! - Content files (.md) following standard frontmatter format

pub mod schema;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A registered feature in the CO system
#[derive(Debug, Clone)]
pub struct Feature {
    /// Feature name (e.g., "agents", "tools")
    pub name: String,
    /// Path to the feature directory
    pub path: PathBuf,
    /// Whether this is a system feature (vs user-defined)
    pub is_system: bool,
    /// Schema for this feature (if schema.yaml exists)
    pub schema: Option<schema::FeatureSchema>,
}

/// Registry of all discovered features
#[derive(Debug, Default)]
pub struct FeatureRegistry {
    /// Map of feature name to Feature
    pub features: HashMap<String, Feature>,
}

impl FeatureRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Discover features from a root directory
    ///
    /// Scans for directories with schema.yaml or known feature names
    pub fn discover(root: &Path) -> Self {
        let mut registry = Self::new();

        // Known system features
        let known_features = ["agents", "tools"];

        for name in &known_features {
            let path = root.join(name);
            if path.is_dir() {
                let schema = schema::FeatureSchema::load(&path).ok();
                registry.features.insert(
                    name.to_string(),
                    Feature {
                        name: name.to_string(),
                        path,
                        is_system: true,
                        schema,
                    },
                );
            }
        }

        // Also check user/ directory for user-defined features
        let user_dir = root.join("user");
        if user_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&user_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let name = path.file_name().unwrap().to_string_lossy().to_string();
                        // Only add if not already registered as system feature
                        if !registry.features.contains_key(&name) {
                            let schema = schema::FeatureSchema::load(&path).ok();
                            registry.features.insert(
                                name.clone(),
                                Feature {
                                    name,
                                    path,
                                    is_system: false,
                                    schema,
                                },
                            );
                        }
                    }
                }
            }
        }

        registry
    }

    /// Get a feature by name
    pub fn get(&self, name: &str) -> Option<&Feature> {
        self.features.get(name)
    }

    /// Check if a feature is registered
    pub fn has(&self, name: &str) -> bool {
        self.features.contains_key(name)
    }

    /// List all feature names
    pub fn names(&self) -> Vec<&str> {
        self.features.keys().map(|s| s.as_str()).collect()
    }
}

impl Feature {
    /// List all items in this feature (scans for .md files)
    pub fn items(&self) -> Vec<PathBuf> {
        let mut items = Vec::new();

        if let Ok(entries) = std::fs::read_dir(&self.path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|e| e == "md") {
                    items.push(path);
                }
            }
        }

        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_feature_dirs(dir: &Path) {
        // Create agents/ with schema.yaml
        let agents_dir = dir.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("schema.yaml"),
            r#"
name: agents
version: 1
properties:
  type:
    kind: text
    required: true
  model:
    kind: text
  capabilities:
    kind: list
"#,
        )
        .unwrap();

        // Create an agent file
        fs::write(
            agents_dir.join("researcher.md"),
            r#"---
type: agent
id: researcher
agent_type: researcher
model: claude-3-opus
capabilities: search, analyze, summarize
---

# Researcher Agent

An agent that researches topics.
"#,
        )
        .unwrap();

        // Create tools/ directory (without schema)
        let tools_dir = dir.join("tools");
        fs::create_dir_all(&tools_dir).unwrap();
    }

    #[test]
    fn test_discover_features_finds_agents_directory() {
        let temp = TempDir::new().unwrap();
        setup_test_feature_dirs(temp.path());

        let registry = FeatureRegistry::discover(temp.path());

        assert!(registry.has("agents"), "Should discover agents feature");
        assert!(registry.has("tools"), "Should discover tools feature");
    }

    #[test]
    fn test_discover_features_loads_schema() {
        let temp = TempDir::new().unwrap();
        setup_test_feature_dirs(temp.path());

        let registry = FeatureRegistry::discover(temp.path());

        let agents = registry.get("agents").expect("agents should exist");
        assert!(agents.schema.is_some(), "agents should have schema loaded");

        let tools = registry.get("tools").expect("tools should exist");
        assert!(tools.schema.is_none(), "tools should not have schema");
    }

    #[test]
    fn test_feature_lists_items() {
        let temp = TempDir::new().unwrap();
        setup_test_feature_dirs(temp.path());

        let registry = FeatureRegistry::discover(temp.path());
        let agents = registry.get("agents").unwrap();

        let items = agents.items();
        assert_eq!(items.len(), 1, "Should find one agent item");
        assert!(items[0].ends_with("researcher.md"));
    }

    #[test]
    fn test_discover_user_features() {
        let temp = TempDir::new().unwrap();
        setup_test_feature_dirs(temp.path());

        // Add a user-defined feature
        let user_agents = temp.path().join("user/agents");
        fs::create_dir_all(&user_agents).unwrap();
        fs::write(
            user_agents.join("custom.md"),
            r#"---
type: agent
id: custom
---
Custom agent
"#,
        )
        .unwrap();

        let registry = FeatureRegistry::discover(temp.path());

        // agents should be from system (not user)
        let agents = registry.get("agents").unwrap();
        assert!(agents.is_system);
    }

    #[test]
    fn test_empty_directory_returns_empty_registry() {
        let temp = TempDir::new().unwrap();
        let registry = FeatureRegistry::discover(temp.path());

        assert!(!registry.has("agents"));
        assert!(!registry.has("tools"));
        assert!(registry.names().is_empty());
    }
}
