//! CO-325: Type category registry — maps content subtypes to their category.
//!
//! Each content type can optionally belong to a category (e.g. `song` belongs
//! to `music`). Querying by category expands to all subtypes in that category.
//!
//! The default registry is built from the schema files in `work/co/schema/`.
//! At runtime (web server) the mapping is embedded; tests can also load from
//! the directory to verify the YAML files stay consistent.

use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// TypeCategoryRegistry
// ---------------------------------------------------------------------------

/// Maps content type categories to their subtypes and vice versa.
///
/// Used by the query DSL to expand `type:music` into `IN ('song', 'album')`.
#[derive(Debug, Default, Clone)]
pub struct TypeCategoryRegistry {
    /// category name → sorted list of subtype names
    pub categories: HashMap<String, Vec<String>>,
    /// type name → category name
    pub type_to_category: HashMap<String, String>,
    /// type name → required frontmatter field names
    pub required_fields: HashMap<String, Vec<String>>,
}

impl TypeCategoryRegistry {
    /// Build the default registry from built-in type definitions.
    ///
    /// This is derived from `work/co/schema/*.yaml`; no I/O involved at
    /// runtime. Tests verify consistency with the actual YAML files.
    pub fn default_registry() -> Self {
        let mut r = Self::default();
        r.register("song", "music", &["title"]);
        r.register("album", "music", &["title"]);
        r.register("poem", "writing", &["title"]);
        r.register("essay", "writing", &["title"]);
        r.register("video", "media", &["title"]);
        r.register("url", "reference", &["title", "url"]);
        r.register("quote", "reference", &["source"]);
        r.register("notas", "reference", &["caderno_id", "pagina"]);
        r
    }

    /// Load a registry from a directory of per-type YAML schema files.
    ///
    /// Each `*.yaml` file must contain at minimum `type` and `category` fields.
    /// Properties with `required: true` are recorded for validation.
    pub fn load_from_dir(schema_dir: &Path) -> Self {
        let mut registry = Self::default();
        let Ok(entries) = std::fs::read_dir(schema_dir) else {
            return registry;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yaml") {
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(schema) = serde_yaml::from_str::<TypeSchemaFile>(&content) else {
                    continue;
                };
                let required: Vec<&str> = schema
                    .properties
                    .iter()
                    .filter_map(|(name, val)| {
                        let is_req = val
                            .as_mapping()
                            .and_then(|m| m.get("required"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if is_req { Some(name.as_str()) } else { None }
                    })
                    .collect();
                registry.register(&schema.type_name, &schema.category, &required);
            }
        }
        registry
    }

    fn register(&mut self, type_name: &str, category: &str, required: &[&str]) {
        self.categories
            .entry(category.to_string())
            .or_default()
            .push(type_name.to_string());
        self.type_to_category
            .insert(type_name.to_string(), category.to_string());
        if !required.is_empty() {
            self.required_fields.insert(
                type_name.to_string(),
                required.iter().map(|s| s.to_string()).collect(),
            );
        }
    }

    /// Expand a category name to its constituent types.
    ///
    /// Returns an empty slice if `category` is not a known category.
    pub fn category_types(&self, category: &str) -> &[String] {
        self.categories
            .get(category)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return `true` if `name` is a registered category (not a type).
    pub fn is_category(&self, name: &str) -> bool {
        self.categories.contains_key(name)
    }

    /// Return `true` if `name` is a registered content type.
    pub fn is_type(&self, name: &str) -> bool {
        self.type_to_category.contains_key(name)
    }

    /// Return the category a type belongs to, or `None`.
    pub fn type_category(&self, type_name: &str) -> Option<&str> {
        self.type_to_category.get(type_name).map(String::as_str)
    }

    /// Required frontmatter fields for a type.
    pub fn required_fields_for(&self, type_name: &str) -> &[String] {
        self.required_fields
            .get(type_name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// All registered type names.
    pub fn all_types(&self) -> Vec<&str> {
        self.type_to_category.keys().map(String::as_str).collect()
    }

    /// All category names.
    pub fn all_categories(&self) -> Vec<&str> {
        self.categories.keys().map(String::as_str).collect()
    }
}

// ---------------------------------------------------------------------------
// YAML schema file format (internal)
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct TypeSchemaFile {
    #[serde(rename = "type")]
    type_name: String,
    category: String,
    #[serde(default)]
    properties: HashMap<String, serde_yaml::Value>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_registry_music_category() {
        let r = TypeCategoryRegistry::default_registry();
        let types = r.category_types("music");
        assert!(
            types.contains(&"song".to_string()),
            "music should contain song"
        );
        assert!(
            types.contains(&"album".to_string()),
            "music should contain album"
        );
    }

    #[test]
    fn test_default_registry_writing_category() {
        let r = TypeCategoryRegistry::default_registry();
        let types = r.category_types("writing");
        assert!(types.contains(&"poem".to_string()));
        assert!(types.contains(&"essay".to_string()));
    }

    #[test]
    fn test_default_registry_reference_category() {
        let r = TypeCategoryRegistry::default_registry();
        let types = r.category_types("reference");
        assert!(types.contains(&"url".to_string()));
        assert!(types.contains(&"quote".to_string()));
        assert!(types.contains(&"notas".to_string()));
    }

    #[test]
    fn test_default_registry_is_category() {
        let r = TypeCategoryRegistry::default_registry();
        assert!(r.is_category("music"));
        assert!(r.is_category("writing"));
        assert!(r.is_category("media"));
        assert!(r.is_category("reference"));
        assert!(!r.is_category("song"), "song is a type, not a category");
    }

    #[test]
    fn test_default_registry_is_type() {
        let r = TypeCategoryRegistry::default_registry();
        assert!(r.is_type("song"));
        assert!(r.is_type("notas"));
        assert!(!r.is_type("music"), "music is a category, not a type");
    }

    #[test]
    fn test_notas_required_fields() {
        let r = TypeCategoryRegistry::default_registry();
        let req = r.required_fields_for("notas");
        assert!(req.contains(&"caderno_id".to_string()));
        assert!(req.contains(&"pagina".to_string()));
    }

    #[test]
    fn test_type_category_lookup() {
        let r = TypeCategoryRegistry::default_registry();
        assert_eq!(r.type_category("song"), Some("music"));
        assert_eq!(r.type_category("notas"), Some("reference"));
        assert_eq!(r.type_category("unknown"), None);
    }

    #[test]
    fn test_load_from_dir() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("song.yaml"),
            "type: song\ncategory: music\nproperties:\n  title:\n    required: true\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("notas.yaml"),
            "type: notas\ncategory: reference\nproperties:\n  caderno_id:\n    required: true\n  pagina:\n    required: true\n",
        )
        .unwrap();

        let r = TypeCategoryRegistry::load_from_dir(dir.path());
        assert!(r.is_type("song"));
        assert!(r.is_category("music"));
        let req = r.required_fields_for("notas");
        assert!(req.contains(&"caderno_id".to_string()));
        assert!(req.contains(&"pagina".to_string()));
    }

    #[test]
    fn test_schema_files_consistent_with_defaults() {
        // Verify that work/co/schema/*.yaml matches the hardcoded defaults.
        let schema_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("work/co/schema");

        if !schema_dir.exists() {
            return; // Skip when running outside the repo
        }

        let from_yaml = TypeCategoryRegistry::load_from_dir(&schema_dir);
        let default = TypeCategoryRegistry::default_registry();

        for type_name in default.all_types() {
            assert!(
                from_yaml.is_type(type_name),
                "Type '{type_name}' from defaults is missing in YAML schemas"
            );
            assert_eq!(
                from_yaml.type_category(type_name),
                default.type_category(type_name),
                "Category mismatch for type '{type_name}'"
            );
        }
    }
}
