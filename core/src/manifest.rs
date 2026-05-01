//! Universe manifest — `_universe.yaml`
//!
//! Every universe ships a `_universe.yaml` at its root that declares content
//! types, presentation hints, doc generators, and relationships.  Sub-tasks
//! CO-71..CO-75 implement against the contract defined here.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MANIFEST_FILENAME: &str = "_universe.yaml";
/// Maximum manifest file size (100 KB).
pub const MAX_MANIFEST_BYTES: usize = 100 * 1024;
/// Maximum number of content types per manifest.
pub const MAX_CONTENT_TYPES: usize = 100;
/// Current supported schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

const KNOWN_TOP_LEVEL_KEYS: &[&str] = &[
    "schema_version",
    "name",
    "content_types",
    "doc_generators",
    "relationships",
    "views",
];

// ---------------------------------------------------------------------------
// Error / warning types
// ---------------------------------------------------------------------------

/// Errors from manifest parsing or validation.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest too large: {size} bytes (max {max})")]
    TooLarge { size: usize, max: usize },

    #[error("too many content types: {count} (max {max})")]
    TooManyContentTypes { count: usize, max: usize },

    #[error("YAML parse error: {0}")]
    Parse(#[from] serde_yaml::Error),

    #[error("invalid field at '{path}': {message}")]
    InvalidField { path: String, message: String },
}

/// A non-fatal warning emitted during manifest validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestWarning {
    /// Dot-notation field path that triggered the warning.
    pub field: String,
    pub message: String,
}

/// Result of a successful manifest parse: the manifest plus any warnings.
#[derive(Debug)]
pub struct ParseResult {
    pub manifest: Manifest,
    pub warnings: Vec<ManifestWarning>,
}

// ---------------------------------------------------------------------------
// Manifest struct hierarchy
// ---------------------------------------------------------------------------

/// Top-level universe manifest (`_universe.yaml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_types: Vec<ContentType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc_generators: Vec<DocGenerator>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<Relationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<View>,
}

/// A content type declared in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentType {
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schema: BTreeMap<String, FieldDef>,
    #[serde(default, skip_serializing_if = "Presentation::is_empty")]
    pub presentation: Presentation,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub indexes: Vec<String>,
}

/// A field definition inside a content type schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    /// Allowed values for [`FieldType::Enum`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// Semantic meaning for [`FieldType::Date`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic: Option<DateSemantic>,
    /// Target content type for [`FieldType::Ref`] / [`FieldType::RefList`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

/// Scalar field types supported in manifest schemas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    String,
    Number,
    Boolean,
    Date,
    Enum,
    Ref,
    RefList,
}

/// Semantic meaning of a date field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateSemantic {
    Due,
    Event,
    Created,
    Updated,
}

/// Presentation hints for a content type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Presentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board: Option<BoardPresentation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list: Option<ListPresentation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar: Option<CalendarPresentation>,
}

impl Presentation {
    fn is_empty(&self) -> bool {
        self.board.is_none() && self.list.is_none() && self.calendar.is_none()
    }
}

/// Kanban board presentation: columns are enum values shown as board columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardPresentation {
    pub columns: Vec<String>,
}

/// List presentation: sort order for the list view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListPresentation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<String>,
}

/// Calendar presentation: which date field drives the calendar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalendarPresentation {
    pub date_field: String,
}

/// An external doc generator (e.g., scaladoc, sphinx).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocGenerator {
    pub format: String,
    pub source_dir: String,
    pub output_type: String,
    pub on: DocGeneratorTrigger,
}

/// When the doc generator runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocGeneratorTrigger {
    Sync,
    Push,
    Manual,
}

/// A declared relationship between content types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relationship {
    /// Source in `content_type.field` notation.
    pub from: String,
    /// Target content type name.
    pub to: String,
    pub cardinality: Cardinality,
}

/// Relationship cardinality.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cardinality {
    OneToOne,
    OneToMany,
    ManyToOne,
    ManyToMany,
}

/// A named view over content types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub name: String,
    #[serde(rename = "type")]
    pub view_type: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_field: Option<String>,
}

// ---------------------------------------------------------------------------
// Manifest impl
// ---------------------------------------------------------------------------

impl Manifest {
    /// Serialize this manifest to a canonical YAML string.
    pub fn to_yaml(&self) -> Result<String, ManifestError> {
        Ok(serde_yaml::to_string(self)?)
    }

    /// Returns `true` if this manifest's `schema_version` is higher than
    /// `stored_version`, meaning CO-71 entry-payload migration must run.
    pub fn triggers_migration_from(&self, stored_version: u32) -> bool {
        self.schema_version > stored_version
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a `_universe.yaml` manifest from raw bytes.
///
/// - Returns [`ManifestError::TooLarge`] if `bytes.len() > MAX_MANIFEST_BYTES`.
/// - Emits [`ManifestWarning`]s for unknown top-level keys (forward-compat).
/// - Returns [`ManifestError::InvalidField`] with a dot-notation path for
///   semantic validation failures.
pub fn parse(bytes: &[u8]) -> Result<ParseResult, ManifestError> {
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge {
            size: bytes.len(),
            max: MAX_MANIFEST_BYTES,
        });
    }

    let raw: serde_yaml::Value = serde_yaml::from_slice(bytes)?;

    let mut warnings = Vec::new();
    if let serde_yaml::Value::Mapping(ref map) = raw {
        for key in map.keys() {
            if let Some(k) = key.as_str()
                && !KNOWN_TOP_LEVEL_KEYS.contains(&k)
            {
                warnings.push(ManifestWarning {
                    field: k.to_string(),
                    message: "unknown top-level key, ignored for forward compatibility".to_string(),
                });
            }
        }
    }

    let manifest: Manifest = serde_yaml::from_value(raw)?;
    validate(&manifest)?;

    Ok(ParseResult { manifest, warnings })
}

/// Parse a `_universe.yaml` manifest from a UTF-8 string.
pub fn parse_str(s: &str) -> Result<ParseResult, ManifestError> {
    parse(s.as_bytes())
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    if manifest.schema_version < 1 {
        return Err(ManifestError::InvalidField {
            path: "schema_version".to_string(),
            message: "must be >= 1".to_string(),
        });
    }

    if manifest.name.trim().is_empty() {
        return Err(ManifestError::InvalidField {
            path: "name".to_string(),
            message: "must not be empty".to_string(),
        });
    }

    if manifest.content_types.len() > MAX_CONTENT_TYPES {
        return Err(ManifestError::TooManyContentTypes {
            count: manifest.content_types.len(),
            max: MAX_CONTENT_TYPES,
        });
    }

    for (i, ct) in manifest.content_types.iter().enumerate() {
        if ct.name.trim().is_empty() {
            return Err(ManifestError::InvalidField {
                path: format!("content_types[{i}].name"),
                message: "must not be empty".to_string(),
            });
        }

        for (field_name, field_def) in &ct.schema {
            let prefix = format!("content_types[{i}].schema.{field_name}");
            match field_def.field_type {
                FieldType::Enum if field_def.values.is_empty() => {
                    return Err(ManifestError::InvalidField {
                        path: format!("{prefix}.values"),
                        message: "enum type requires at least one value".to_string(),
                    });
                }
                FieldType::Ref | FieldType::RefList if field_def.target.is_none() => {
                    return Err(ManifestError::InvalidField {
                        path: format!("{prefix}.target"),
                        message: "ref/ref_list type requires a target content type".to_string(),
                    });
                }
                _ => {}
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Default manifest (legacy universes: board-of-tasks)
// ---------------------------------------------------------------------------

/// Return the default manifest for a legacy universe.
///
/// Produces a board with a single `task` content type and
/// `[todo, doing, done]` columns — identical to pre-manifest behaviour.
pub fn default_manifest(name: impl Into<String>) -> Manifest {
    let mut schema = BTreeMap::new();
    schema.insert(
        "status".to_string(),
        FieldDef {
            field_type: FieldType::Enum,
            required: false,
            values: vec!["todo".to_string(), "doing".to_string(), "done".to_string()],
            semantic: None,
            target: None,
        },
    );
    schema.insert(
        "title".to_string(),
        FieldDef {
            field_type: FieldType::String,
            required: true,
            values: vec![],
            semantic: None,
            target: None,
        },
    );

    Manifest {
        schema_version: 1,
        name: name.into(),
        content_types: vec![ContentType {
            name: "task".to_string(),
            schema,
            presentation: Presentation {
                board: Some(BoardPresentation {
                    columns: vec!["todo".to_string(), "doing".to_string(), "done".to_string()],
                }),
                list: None,
                calendar: None,
            },
            indexes: vec!["status".to_string()],
        }],
        doc_generators: vec![],
        relationships: vec![],
        views: vec![],
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_false(b: &bool) -> bool {
    !b
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_MANIFEST_YAML: &str = r#"schema_version: 1
name: ArteLonga
content_types:
  - name: tarefa
    schema:
      due_at:
        type: date
        semantic: due
      status:
        type: enum
        values:
          - todo
          - doing
          - done
      title:
        type: string
        required: true
    presentation:
      board:
        columns:
          - todo
          - doing
          - done
      list:
        sort:
          - due_at
          - title
    indexes:
      - status
      - due_at
  - name: evento
    schema:
      attendees:
        type: ref_list
        target: pessoa
      event_at:
        type: date
        semantic: event
      title:
        type: string
        required: true
    presentation:
      calendar:
        date_field: event_at
    indexes:
      - event_at
doc_generators:
  - format: scaladoc
    source_dir: src/main/scala
    output_type: doc.scala
    on: sync
relationships:
  - from: tarefa.assignee
    to: pessoa
    cardinality: many-to-one
views:
  - name: roadmap
    type: gantt
    source: tarefa
    date_start: created_at
    date_end: due_at
"#;

    #[test]
    fn test_parse_valid_full_manifest() {
        let result = parse_str(FULL_MANIFEST_YAML).expect("should parse valid manifest");
        assert!(result.warnings.is_empty());

        let m = &result.manifest;
        assert_eq!(m.schema_version, 1);
        assert_eq!(m.name, "ArteLonga");
        assert_eq!(m.content_types.len(), 2);
        assert_eq!(m.content_types[0].name, "tarefa");
        assert_eq!(m.content_types[1].name, "evento");
        assert_eq!(m.doc_generators.len(), 1);
        assert_eq!(m.relationships.len(), 1);
        assert_eq!(m.views.len(), 1);
    }

    #[test]
    fn test_parse_minimal_manifest() {
        let yaml = "schema_version: 1\nname: MyUniverse\n";
        let result = parse_str(yaml).expect("should parse minimal manifest");
        assert_eq!(result.manifest.schema_version, 1);
        assert_eq!(result.manifest.name, "MyUniverse");
        assert!(result.manifest.content_types.is_empty());
    }

    #[test]
    fn test_parse_too_large() {
        let large = "a".repeat(MAX_MANIFEST_BYTES + 1);
        let err = parse(large.as_bytes()).unwrap_err();
        assert!(
            matches!(err, ManifestError::TooLarge { .. }),
            "expected TooLarge, got: {err}"
        );
    }

    #[test]
    fn test_parse_too_many_content_types() {
        let types: String = (0..=MAX_CONTENT_TYPES)
            .map(|i| format!("  - name: type{i}\n"))
            .collect();
        let yaml = format!("schema_version: 1\nname: X\ncontent_types:\n{types}");
        let err = parse_str(&yaml).unwrap_err();
        assert!(
            matches!(err, ManifestError::TooManyContentTypes { .. }),
            "expected TooManyContentTypes, got: {err}"
        );
    }

    #[test]
    fn test_unknown_top_level_keys_are_warnings() {
        let yaml = "schema_version: 1\nname: X\nfuture_feature: true\nanother_key: 42\n";
        let result = parse_str(yaml).expect("should parse with warnings");
        assert_eq!(result.warnings.len(), 2);
        let fields: Vec<_> = result.warnings.iter().map(|w| w.field.as_str()).collect();
        assert!(fields.contains(&"future_feature"));
        assert!(fields.contains(&"another_key"));
    }

    #[test]
    fn test_known_keys_produce_no_warnings() {
        let result = parse_str(FULL_MANIFEST_YAML).expect("should parse");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_empty_name_rejected() {
        let yaml = "schema_version: 1\nname: ''\n";
        let err = parse_str(yaml).unwrap_err();
        assert!(
            matches!(&err, ManifestError::InvalidField { path, .. } if path == "name"),
            "expected InvalidField at 'name', got: {err}"
        );
    }

    #[test]
    fn test_whitespace_name_rejected() {
        let yaml = "schema_version: 1\nname: '   '\n";
        let err = parse_str(yaml).unwrap_err();
        assert!(matches!(&err, ManifestError::InvalidField { path, .. } if path == "name"));
    }

    #[test]
    fn test_zero_schema_version_rejected() {
        let yaml = "schema_version: 0\nname: X\n";
        let err = parse_str(yaml).unwrap_err();
        assert!(
            matches!(&err, ManifestError::InvalidField { path, .. } if path == "schema_version"),
            "got: {err}"
        );
    }

    #[test]
    fn test_enum_without_values_rejected() {
        let yaml = r#"schema_version: 1
name: X
content_types:
  - name: tarefa
    schema:
      status:
        type: enum
"#;
        let err = parse_str(yaml).unwrap_err();
        assert!(
            matches!(&err, ManifestError::InvalidField { path, .. }
                if path.contains("status") && path.ends_with(".values")),
            "got: {err}"
        );
    }

    #[test]
    fn test_ref_without_target_rejected() {
        let yaml = r#"schema_version: 1
name: X
content_types:
  - name: tarefa
    schema:
      assignee:
        type: ref
"#;
        let err = parse_str(yaml).unwrap_err();
        assert!(
            matches!(&err, ManifestError::InvalidField { path, .. }
                if path.contains("assignee") && path.ends_with(".target")),
            "got: {err}"
        );
    }

    #[test]
    fn test_ref_list_without_target_rejected() {
        let yaml = r#"schema_version: 1
name: X
content_types:
  - name: evento
    schema:
      attendees:
        type: ref_list
"#;
        let err = parse_str(yaml).unwrap_err();
        assert!(
            matches!(&err, ManifestError::InvalidField { path, .. }
                if path.contains("attendees") && path.ends_with(".target")),
            "got: {err}"
        );
    }

    #[test]
    fn test_error_path_includes_index() {
        let yaml = r#"schema_version: 1
name: X
content_types:
  - name: first
  - name: second
    schema:
      broken:
        type: enum
"#;
        let err = parse_str(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("content_types[1]"),
            "error path should reference index 1, got: {msg}"
        );
    }

    #[test]
    fn test_round_trip() {
        let result = parse_str(FULL_MANIFEST_YAML).expect("initial parse");
        let yaml2 = result.manifest.to_yaml().expect("to_yaml");
        let result2 = parse_str(&yaml2).expect("re-parse");
        assert_eq!(
            result.manifest, result2.manifest,
            "round-trip must preserve struct"
        );
    }

    #[test]
    fn test_default_manifest_is_valid() {
        let m = default_manifest("MyUniverse");
        let yaml = m.to_yaml().expect("to_yaml");
        let result = parse_str(&yaml).expect("default manifest must parse");
        assert_eq!(result.manifest, m);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_default_manifest_has_task_board() {
        let m = default_manifest("test");
        assert_eq!(m.content_types.len(), 1);
        let ct = &m.content_types[0];
        assert_eq!(ct.name, "task");
        let board = ct.presentation.board.as_ref().expect("board should exist");
        assert_eq!(board.columns, ["todo", "doing", "done"]);
        assert!(ct.schema.contains_key("title"));
        assert!(ct.schema.contains_key("status"));
        let status = &ct.schema["status"];
        assert_eq!(status.field_type, FieldType::Enum);
        assert_eq!(status.values, ["todo", "doing", "done"]);
    }

    #[test]
    fn test_default_manifest_canonical_round_trip() {
        let m = default_manifest("CanonicalTest");
        let yaml1 = m.to_yaml().expect("first serialize");
        let yaml2 = parse_str(&yaml1)
            .expect("re-parse")
            .manifest
            .to_yaml()
            .expect("second serialize");
        assert_eq!(
            yaml1, yaml2,
            "canonical round-trip must produce identical bytes"
        );
    }

    #[test]
    fn test_triggers_migration_from() {
        let m = Manifest {
            schema_version: 2,
            name: "X".to_string(),
            content_types: vec![],
            doc_generators: vec![],
            relationships: vec![],
            views: vec![],
        };
        assert!(m.triggers_migration_from(1));
        assert!(!m.triggers_migration_from(2));
        assert!(!m.triggers_migration_from(3));
    }

    #[test]
    fn test_cardinality_serialization() {
        let rel = Relationship {
            from: "tarefa.assignee".to_string(),
            to: "pessoa".to_string(),
            cardinality: Cardinality::ManyToOne,
        };
        let yaml = serde_yaml::to_string(&rel).unwrap();
        assert!(
            yaml.contains("many-to-one"),
            "cardinality must serialize as kebab-case: {yaml}"
        );
    }

    #[test]
    fn test_field_type_ref_list_serialization() {
        let fd = FieldDef {
            field_type: FieldType::RefList,
            required: false,
            values: vec![],
            semantic: None,
            target: Some("pessoa".to_string()),
        };
        let yaml = serde_yaml::to_string(&fd).unwrap();
        assert!(
            yaml.contains("ref_list"),
            "FieldType::RefList must serialize as ref_list: {yaml}"
        );
    }
}
