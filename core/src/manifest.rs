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
    "parent",
    "surface_dns",
    "visibility",
    "content_types",
    "doc_generators",
    "relationships",
    "views",
    "properties_per_type",
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

// ---------------------------------------------------------------------------
// CO-156: properties_per_type — per-content-type property declarations using
// the lightweight `kind: text|int|enum|list` vocabulary.
// ---------------------------------------------------------------------------

/// Property kind in the `properties_per_type` YAML vocabulary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropKind {
    Text,
    Int,
    Enum,
    List,
}

/// A property definition inside `properties_per_type.<type>.<field>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropDef {
    pub kind: PropKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Manifest struct
// ---------------------------------------------------------------------------

/// Top-level universe manifest (`_universe.yaml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub name: String,
    /// CO-338: parent universe key (lineage). `None` = top-level / root.
    /// Surfaces the recursive sub-universe ⇄ universe relationship in the
    /// manifest so `key::path` refs can walk to a deployable ancestor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// CO-338: deployment DNS host — set only on deployable units (e.g.
    /// `yggdrasil.artelonga.com.br`). `None` = inherits a deploying ancestor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_dns: Option<String>,
    /// CO-467: declared visibility (`private` | `public-subscribable` |
    /// `public-static` | `unlisted`). `None` = inherit/default `private`. The
    /// workspace scan honors this on register AND reconciles existing rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_types: Vec<ContentType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc_generators: Vec<DocGenerator>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<Relationship>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<View>,
    /// CO-156: per-content-type property declarations using the `kind` vocabulary.
    /// Merged into `content_types` schemas during parsing; not serialised on output.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties_per_type: BTreeMap<String, BTreeMap<String, PropDef>>,
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
    /// CO-75: template for an auto-generated changelog line when an entry of
    /// this type is created/changed. `{field}` tokens are substituted from the
    /// entry's frontmatter (e.g. `"{title} marcado como {status}"`); `{path}`
    /// and `{title}` are always available. `None` → a default line (title/path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog_summary: Option<String>,
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
    /// Deadline for action.
    DueAt,
    /// When something happens.
    EventAt,
    /// Auto, immutable.
    CreatedAt,
    /// Auto, mutable.
    UpdatedAt,
    /// Planned start.
    ScheduledAt,
    /// Public-facing publish date.
    PublishedAt,
    /// When content becomes invalid.
    ExpiresAt,
    /// When this version takes effect.
    EffectiveAt,
}

impl DateSemantic {
    /// Canonical string identifier (matches the query-param value).
    pub fn as_str(&self) -> &'static str {
        match self {
            DateSemantic::DueAt => "due_at",
            DateSemantic::EventAt => "event_at",
            DateSemantic::CreatedAt => "created_at",
            DateSemantic::UpdatedAt => "updated_at",
            DateSemantic::ScheduledAt => "scheduled_at",
            DateSemantic::PublishedAt => "published_at",
            DateSemantic::ExpiresAt => "expires_at",
            DateSemantic::EffectiveAt => "effective_at",
        }
    }
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

    // CO-156: normalise content_types so string entries (e.g. `- reference`) are
    // expanded to full ContentType objects before serde_yaml deserialises them.
    let raw = normalize_content_type_strings(raw);

    let mut manifest: Manifest = serde_yaml::from_value(raw)?;

    // CO-156: merge properties_per_type into content_types schemas.
    merge_properties_per_type(&mut manifest);

    validate(&manifest)?;

    Ok(ParseResult { manifest, warnings })
}

/// Convert any bare-string items inside `content_types:` to `{name: <string>}` mappings
/// so the standard ContentType deserialiser can handle them.
fn normalize_content_type_strings(mut raw: serde_yaml::Value) -> serde_yaml::Value {
    if let serde_yaml::Value::Mapping(ref mut map) = raw {
        let key = serde_yaml::Value::String("content_types".to_string());
        if let Some(serde_yaml::Value::Sequence(seq)) = map.get_mut(&key) {
            for item in seq.iter_mut() {
                if let serde_yaml::Value::String(ref name) = item.clone() {
                    let mut ct_map = serde_yaml::Mapping::new();
                    ct_map.insert(
                        serde_yaml::Value::String("name".to_string()),
                        serde_yaml::Value::String(name.clone()),
                    );
                    *item = serde_yaml::Value::Mapping(ct_map);
                }
            }
        }
    }
    raw
}

/// Merge `properties_per_type` entries into the corresponding ContentType schemas.
///
/// - If a content type named by the key already exists in `content_types`, its
///   schema is augmented (existing fields are NOT overwritten).
/// - If no matching ContentType exists, a new one is created and appended.
fn merge_properties_per_type(manifest: &mut Manifest) {
    if manifest.properties_per_type.is_empty() {
        return;
    }
    for (type_name, props) in &manifest.properties_per_type.clone() {
        let ct = if let Some(ct) = manifest
            .content_types
            .iter_mut()
            .find(|ct| ct.name == *type_name)
        {
            ct
        } else {
            manifest.content_types.push(ContentType {
                name: type_name.clone(),
                schema: BTreeMap::new(),
                presentation: Presentation::default(),
                indexes: vec![],
                changelog_summary: None,
            });
            manifest.content_types.last_mut().unwrap()
        };

        for (field_name, prop_def) in props {
            ct.schema
                .entry(field_name.clone())
                .or_insert_with(|| prop_def_to_field_def(prop_def));
        }
    }
}

fn prop_def_to_field_def(prop: &PropDef) -> FieldDef {
    let field_type = match prop.kind {
        PropKind::Text => FieldType::String,
        PropKind::Int => FieldType::Number,
        PropKind::Enum => FieldType::Enum,
        PropKind::List => FieldType::String,
    };
    FieldDef {
        field_type,
        required: prop.required,
        values: prop.values.clone(),
        semantic: None,
        target: None,
    }
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
/// Delivery pipeline status values (CO-398 — default for new project universes).
pub const DELIVERY_PIPELINE_STATUSES: &[&str] =
    &["todo", "started", "in_progress", "review", "done"];

pub fn default_manifest(name: impl Into<String>) -> Manifest {
    let statuses: Vec<String> = DELIVERY_PIPELINE_STATUSES
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut schema = BTreeMap::new();
    schema.insert(
        "status".to_string(),
        FieldDef {
            field_type: FieldType::Enum,
            required: false,
            values: statuses.clone(),
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
    schema.insert(
        "pr_url".to_string(),
        FieldDef {
            field_type: FieldType::String,
            required: false,
            values: vec![],
            semantic: None,
            target: None,
        },
    );
    schema.insert(
        "preview_url".to_string(),
        FieldDef {
            field_type: FieldType::String,
            required: false,
            values: vec![],
            semantic: None,
            target: None,
        },
    );

    Manifest {
        schema_version: 1,
        name: name.into(),
        parent: None,
        surface_dns: None,
        visibility: None,
        content_types: vec![ContentType {
            name: "task".to_string(),
            schema,
            presentation: Presentation {
                board: Some(BoardPresentation { columns: statuses }),
                list: None,
                calendar: None,
            },
            indexes: vec!["status".to_string()],
            changelog_summary: None,
        }],
        doc_generators: vec![],
        relationships: vec![],
        views: vec![],
        properties_per_type: BTreeMap::new(),
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
        semantic: due_at
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
        semantic: event_at
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
    fn test_parse_changelog_summary_template() {
        // CO-75: a content type may declare a changelog_summary template that
        // round-trips through parse and serialize.
        let yaml = "schema_version: 1\nname: U\ncontent_types:\n  - name: tarefa\n    changelog_summary: \"{title} marcado como {status}\"\n";
        let m = parse_str(yaml).expect("should parse").manifest;
        assert_eq!(
            m.content_types[0].changelog_summary.as_deref(),
            Some("{title} marcado como {status}")
        );
        // Absent → None (backward compatible with manifests written before CO-75).
        let bare = parse_str("schema_version: 1\nname: U\ncontent_types:\n  - name: nota\n")
            .unwrap()
            .manifest;
        assert_eq!(bare.content_types[0].changelog_summary, None);
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
        assert_eq!(
            board.columns,
            ["todo", "started", "in_progress", "review", "done"]
        );
        assert!(ct.schema.contains_key("title"));
        assert!(ct.schema.contains_key("status"));
        assert!(ct.schema.contains_key("pr_url"));
        assert!(ct.schema.contains_key("preview_url"));
        let status = &ct.schema["status"];
        assert_eq!(status.field_type, FieldType::Enum);
        assert_eq!(
            status.values,
            ["todo", "started", "in_progress", "review", "done"]
        );
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
            parent: None,
            surface_dns: None,
            visibility: None,
            content_types: vec![],
            doc_generators: vec![],
            relationships: vec![],
            views: vec![],
            properties_per_type: BTreeMap::new(),
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
    fn test_date_semantic_as_str_all_variants() {
        assert_eq!(DateSemantic::DueAt.as_str(), "due_at");
        assert_eq!(DateSemantic::EventAt.as_str(), "event_at");
        assert_eq!(DateSemantic::CreatedAt.as_str(), "created_at");
        assert_eq!(DateSemantic::UpdatedAt.as_str(), "updated_at");
        assert_eq!(DateSemantic::ScheduledAt.as_str(), "scheduled_at");
        assert_eq!(DateSemantic::PublishedAt.as_str(), "published_at");
        assert_eq!(DateSemantic::ExpiresAt.as_str(), "expires_at");
        assert_eq!(DateSemantic::EffectiveAt.as_str(), "effective_at");
    }

    #[test]
    fn test_date_semantic_serde_round_trip() {
        let yaml = "schema_version: 1\nname: X\ncontent_types:\n  - name: ev\n    schema:\n      event_at:\n        type: date\n        semantic: event_at\n      due_at:\n        type: date\n        semantic: due_at\n      sched:\n        type: date\n        semantic: scheduled_at\n      pub_at:\n        type: date\n        semantic: published_at\n      exp:\n        type: date\n        semantic: expires_at\n      eff:\n        type: date\n        semantic: effective_at\n";
        let result = parse_str(yaml).expect("should parse all semantics");
        let ct = &result.manifest.content_types[0];
        assert_eq!(ct.schema["event_at"].semantic, Some(DateSemantic::EventAt));
        assert_eq!(ct.schema["due_at"].semantic, Some(DateSemantic::DueAt));
        assert_eq!(ct.schema["sched"].semantic, Some(DateSemantic::ScheduledAt));
        assert_eq!(
            ct.schema["pub_at"].semantic,
            Some(DateSemantic::PublishedAt)
        );
        assert_eq!(ct.schema["exp"].semantic, Some(DateSemantic::ExpiresAt));
        assert_eq!(ct.schema["eff"].semantic, Some(DateSemantic::EffectiveAt));
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

    // CO-156: properties_per_type tests
    #[test]
    fn test_parse_properties_per_type_string_content_types() {
        let yaml = r#"schema_version: 1
name: Topologia
content_types:
  - term
  - reference
properties_per_type:
  reference:
    file:
      kind: text
      description: Sibling asset filename.
    mime:
      kind: text
      required: true
    medium:
      kind: enum
      values: [pdf, image, video, audio, web, citation]
      required: true
    seed_status:
      kind: enum
      values: [stub, reviewed, native-confirmed]
      required: true
    authors:
      kind: list
    size_bytes:
      kind: int
"#;
        let result = parse_str(yaml).expect("should parse properties_per_type");
        assert!(result.warnings.is_empty());
        let m = &result.manifest;
        assert_eq!(m.content_types.len(), 2);

        let term_ct = m.content_types.iter().find(|ct| ct.name == "term").unwrap();
        assert!(term_ct.schema.is_empty(), "term has no properties declared");

        let ref_ct = m
            .content_types
            .iter()
            .find(|ct| ct.name == "reference")
            .unwrap();
        assert!(ref_ct.schema.contains_key("file"));
        assert!(ref_ct.schema.contains_key("mime"));
        assert!(ref_ct.schema.contains_key("medium"));

        let medium = &ref_ct.schema["medium"];
        assert_eq!(medium.field_type, FieldType::Enum);
        assert!(medium.required);
        assert!(medium.values.contains(&"pdf".to_string()));
        assert!(medium.values.contains(&"video".to_string()));

        let mime = &ref_ct.schema["mime"];
        assert_eq!(mime.field_type, FieldType::String);
        assert!(mime.required);

        let authors = &ref_ct.schema["authors"];
        assert_eq!(authors.field_type, FieldType::String, "list maps to String");
        assert!(!authors.required);

        let size = &ref_ct.schema["size_bytes"];
        assert_eq!(size.field_type, FieldType::Number);
    }

    #[test]
    fn test_parse_properties_per_type_creates_missing_content_type() {
        let yaml = r#"schema_version: 1
name: Test
properties_per_type:
  reference:
    mime:
      kind: text
      required: true
    medium:
      kind: enum
      values: [pdf]
      required: true
"#;
        let result = parse_str(yaml).expect("should parse");
        let m = &result.manifest;
        let ref_ct = m
            .content_types
            .iter()
            .find(|ct| ct.name == "reference")
            .unwrap();
        assert!(ref_ct.schema.contains_key("mime"));
    }

    #[test]
    fn test_parse_properties_per_type_no_warning() {
        let yaml = "schema_version: 1\nname: X\nproperties_per_type:\n  reference:\n    mime:\n      kind: text\n      required: true\n    medium:\n      kind: enum\n      values: [pdf]\n      required: true\n";
        let result = parse_str(yaml).expect("should parse");
        assert!(
            result.warnings.is_empty(),
            "properties_per_type must not produce warnings"
        );
    }
}
