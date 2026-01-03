//! Feature schema definition and validation
//!
//! Schemas define property types (not allowed values) following Obsidian-style properties.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Property kinds for feature schemas
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PropertyKind {
    /// Plain string
    #[default]
    Text,
    /// Numeric value
    Number,
    /// ISO date
    Date,
    /// Multiple values (comma-separated or YAML list)
    List,
    /// URL/reference
    Link,
    /// Boolean
    Checkbox,
}

/// A property definition in a schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyDef {
    /// The kind of property
    #[serde(default)]
    pub kind: PropertyKind,
    /// Whether this property is required
    #[serde(default)]
    pub required: bool,
}

/// Schema for a feature type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureSchema {
    /// Feature name
    pub name: String,
    /// Schema version
    #[serde(default = "default_version")]
    pub version: u32,
    /// Property definitions
    #[serde(default)]
    pub properties: HashMap<String, PropertyDef>,
}

fn default_version() -> u32 {
    1
}

impl FeatureSchema {
    /// Load schema from a feature directory
    pub fn load(dir: &Path) -> Result<Self, SchemaError> {
        let schema_path = dir.join("schema.yaml");
        if !schema_path.exists() {
            return Err(SchemaError::NotFound(schema_path));
        }

        let content = std::fs::read_to_string(&schema_path)
            .map_err(|e| SchemaError::Io(schema_path.clone(), e))?;

        serde_yaml::from_str(&content).map_err(|e| SchemaError::Parse(schema_path, e))
    }

    /// Save schema to a feature directory
    pub fn save(&self, dir: &Path) -> Result<(), SchemaError> {
        let schema_path = dir.join("schema.yaml");
        let content =
            serde_yaml::to_string(self).map_err(|e| SchemaError::Parse(schema_path.clone(), e))?;
        std::fs::write(&schema_path, content).map_err(|e| SchemaError::Io(schema_path, e))
    }

    /// Add a property to the schema
    pub fn add_property(&mut self, name: String, kind: PropertyKind, required: bool) {
        self.properties.insert(name, PropertyDef { kind, required });
        self.version += 1;
    }

    /// Remove a property from the schema
    pub fn remove_property(&mut self, name: &str) -> bool {
        if self.properties.remove(name).is_some() {
            self.version += 1;
            true
        } else {
            false
        }
    }

    /// Modify a property's attributes
    pub fn modify_property(
        &mut self,
        name: &str,
        kind: Option<PropertyKind>,
        required: Option<bool>,
    ) -> bool {
        if let Some(prop) = self.properties.get_mut(name) {
            if let Some(k) = kind {
                prop.kind = k;
            }
            if let Some(r) = required {
                prop.required = r;
            }
            self.version += 1;
            true
        } else {
            false
        }
    }

    /// Get property count
    pub fn property_count(&self) -> usize {
        self.properties.len()
    }

    /// Validate a frontmatter map against this schema
    pub fn validate(
        &self,
        frontmatter: &HashMap<String, serde_yaml::Value>,
    ) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        // Check required properties
        for (name, prop) in &self.properties {
            if prop.required && !frontmatter.contains_key(name) {
                errors.push(ValidationError::MissingRequired(name.clone()));
            }
        }

        // Validate property types
        for (name, value) in frontmatter {
            if let Some(prop) = self.properties.get(name)
                && let Some(error) = self.validate_property_type(name, value, &prop.kind)
            {
                errors.push(error);
            }
            // Unknown properties are allowed (extensible)
        }

        errors
    }

    fn validate_property_type(
        &self,
        name: &str,
        value: &serde_yaml::Value,
        kind: &PropertyKind,
    ) -> Option<ValidationError> {
        match kind {
            PropertyKind::Text => {
                if !value.is_string() && !value.is_null() {
                    return Some(ValidationError::TypeMismatch {
                        property: name.to_string(),
                        expected: "text".to_string(),
                        got: value_type_name(value),
                    });
                }
            }
            PropertyKind::Number => {
                if !value.is_number() && !value.is_null() {
                    return Some(ValidationError::TypeMismatch {
                        property: name.to_string(),
                        expected: "number".to_string(),
                        got: value_type_name(value),
                    });
                }
            }
            PropertyKind::List => {
                // Lists can be YAML sequences or comma-separated strings
                if !value.is_sequence() && !value.is_string() && !value.is_null() {
                    return Some(ValidationError::TypeMismatch {
                        property: name.to_string(),
                        expected: "list".to_string(),
                        got: value_type_name(value),
                    });
                }
            }
            PropertyKind::Checkbox => {
                if !value.is_bool() && !value.is_null() {
                    return Some(ValidationError::TypeMismatch {
                        property: name.to_string(),
                        expected: "checkbox".to_string(),
                        got: value_type_name(value),
                    });
                }
            }
            PropertyKind::Date | PropertyKind::Link => {
                // These are stored as strings, so just check for string
                if !value.is_string() && !value.is_null() {
                    return Some(ValidationError::TypeMismatch {
                        property: name.to_string(),
                        expected: format!("{:?}", kind).to_lowercase(),
                        got: value_type_name(value),
                    });
                }
            }
        }
        None
    }
}

fn value_type_name(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::Null => "null".to_string(),
        serde_yaml::Value::Bool(_) => "boolean".to_string(),
        serde_yaml::Value::Number(_) => "number".to_string(),
        serde_yaml::Value::String(_) => "string".to_string(),
        serde_yaml::Value::Sequence(_) => "list".to_string(),
        serde_yaml::Value::Mapping(_) => "object".to_string(),
        serde_yaml::Value::Tagged(_) => "tagged".to_string(),
    }
}

/// Errors that can occur when loading a schema
#[derive(Debug)]
pub enum SchemaError {
    /// Schema file not found
    NotFound(std::path::PathBuf),
    /// IO error reading schema
    Io(std::path::PathBuf, std::io::Error),
    /// Parse error in schema YAML
    Parse(std::path::PathBuf, serde_yaml::Error),
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::NotFound(path) => write!(f, "Schema not found: {}", path.display()),
            SchemaError::Io(path, e) => write!(f, "Error reading {}: {}", path.display(), e),
            SchemaError::Parse(path, e) => write!(f, "Error parsing {}: {}", path.display(), e),
        }
    }
}

impl std::error::Error for SchemaError {}

/// Validation errors for feature items
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Required property is missing
    MissingRequired(String),
    /// Property type doesn't match schema
    TypeMismatch {
        property: String,
        expected: String,
        got: String,
    },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::MissingRequired(name) => {
                write!(f, "Missing required property: {}", name)
            }
            ValidationError::TypeMismatch {
                property,
                expected,
                got,
            } => {
                write!(
                    f,
                    "Property '{}': expected {}, got {}",
                    property, expected, got
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_schema(dir: &Path) {
        std::fs::write(
            dir.join("schema.yaml"),
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
  priority:
    kind: number
  enabled:
    kind: checkbox
"#,
        )
        .unwrap();
    }

    #[test]
    fn test_load_schema() {
        let temp = TempDir::new().unwrap();
        create_test_schema(temp.path());

        let schema = FeatureSchema::load(temp.path()).expect("Should load schema");

        assert_eq!(schema.name, "agents");
        assert_eq!(schema.version, 1);
        assert!(schema.properties.contains_key("type"));
        assert!(schema.properties.contains_key("model"));
    }

    #[test]
    fn test_schema_not_found() {
        let temp = TempDir::new().unwrap();
        let result = FeatureSchema::load(temp.path());

        assert!(matches!(result, Err(SchemaError::NotFound(_))));
    }

    #[test]
    fn test_property_kinds() {
        let temp = TempDir::new().unwrap();
        create_test_schema(temp.path());

        let schema = FeatureSchema::load(temp.path()).unwrap();

        assert_eq!(
            schema.properties.get("type").unwrap().kind,
            PropertyKind::Text
        );
        assert_eq!(
            schema.properties.get("capabilities").unwrap().kind,
            PropertyKind::List
        );
        assert_eq!(
            schema.properties.get("priority").unwrap().kind,
            PropertyKind::Number
        );
        assert_eq!(
            schema.properties.get("enabled").unwrap().kind,
            PropertyKind::Checkbox
        );
    }

    #[test]
    fn test_required_properties() {
        let temp = TempDir::new().unwrap();
        create_test_schema(temp.path());

        let schema = FeatureSchema::load(temp.path()).unwrap();

        assert!(schema.properties.get("type").unwrap().required);
        assert!(!schema.properties.get("model").unwrap().required);
    }

    #[test]
    fn test_validate_missing_required() {
        let temp = TempDir::new().unwrap();
        create_test_schema(temp.path());

        let schema = FeatureSchema::load(temp.path()).unwrap();

        // Frontmatter missing required 'type' property
        let frontmatter: HashMap<String, serde_yaml::Value> = HashMap::new();
        let errors = schema.validate(&frontmatter);

        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            ValidationError::MissingRequired(name) if name == "type"
        ));
    }

    #[test]
    fn test_validate_type_mismatch() {
        let temp = TempDir::new().unwrap();
        create_test_schema(temp.path());

        let schema = FeatureSchema::load(temp.path()).unwrap();

        // priority should be number, not string
        let mut frontmatter: HashMap<String, serde_yaml::Value> = HashMap::new();
        frontmatter.insert(
            "type".to_string(),
            serde_yaml::Value::String("agent".to_string()),
        );
        frontmatter.insert(
            "priority".to_string(),
            serde_yaml::Value::String("high".to_string()),
        );

        let errors = schema.validate(&frontmatter);

        assert_eq!(errors.len(), 1);
        assert!(matches!(
            &errors[0],
            ValidationError::TypeMismatch { property, expected, .. }
            if property == "priority" && expected == "number"
        ));
    }

    #[test]
    fn test_validate_list_accepts_string() {
        let temp = TempDir::new().unwrap();
        create_test_schema(temp.path());

        let schema = FeatureSchema::load(temp.path()).unwrap();

        // capabilities as comma-separated string should be valid
        let mut frontmatter: HashMap<String, serde_yaml::Value> = HashMap::new();
        frontmatter.insert(
            "type".to_string(),
            serde_yaml::Value::String("agent".to_string()),
        );
        frontmatter.insert(
            "capabilities".to_string(),
            serde_yaml::Value::String("search, analyze".to_string()),
        );

        let errors = schema.validate(&frontmatter);
        assert!(
            errors.is_empty(),
            "Comma-separated string should be valid for list"
        );
    }

    #[test]
    fn test_validate_list_accepts_sequence() {
        let temp = TempDir::new().unwrap();
        create_test_schema(temp.path());

        let schema = FeatureSchema::load(temp.path()).unwrap();

        // capabilities as YAML list should be valid
        let mut frontmatter: HashMap<String, serde_yaml::Value> = HashMap::new();
        frontmatter.insert(
            "type".to_string(),
            serde_yaml::Value::String("agent".to_string()),
        );
        frontmatter.insert(
            "capabilities".to_string(),
            serde_yaml::Value::Sequence(vec![
                serde_yaml::Value::String("search".to_string()),
                serde_yaml::Value::String("analyze".to_string()),
            ]),
        );

        let errors = schema.validate(&frontmatter);
        assert!(errors.is_empty(), "YAML sequence should be valid for list");
    }

    #[test]
    fn test_unknown_properties_allowed() {
        let temp = TempDir::new().unwrap();
        create_test_schema(temp.path());

        let schema = FeatureSchema::load(temp.path()).unwrap();

        // Extra properties not in schema should be allowed
        let mut frontmatter: HashMap<String, serde_yaml::Value> = HashMap::new();
        frontmatter.insert(
            "type".to_string(),
            serde_yaml::Value::String("agent".to_string()),
        );
        frontmatter.insert(
            "custom_field".to_string(),
            serde_yaml::Value::String("custom value".to_string()),
        );

        let errors = schema.validate(&frontmatter);
        assert!(errors.is_empty(), "Unknown properties should be allowed");
    }
}
