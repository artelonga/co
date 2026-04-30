//! Payload validation and typed view layer for CO entries.
//!
//! CO-71: every entry write validates its frontmatter JSON against the
//! declaring universe's manifest [`ContentType`] schema.  At read time,
//! fields are coerced to typed Rust values via [`coerce_payload`].

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

use crate::manifest::{ContentType, FieldType};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// A validation error carrying the dot-notation field path that failed.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("field '{path}': {message}")]
pub struct PayloadError {
    pub path: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Typed value
// ---------------------------------------------------------------------------

/// A typed field value coerced from a JSON payload per the manifest schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TypedValue {
    Date(DateTime<Utc>),
    Number(f64),
    Boolean(bool),
    StringArray(Vec<String>),
    String(String),
    Null,
}

// ---------------------------------------------------------------------------
// Typed entry
// ---------------------------------------------------------------------------

/// An entry with its frontmatter fields coerced to typed Rust values.
///
/// Built from a raw JSON payload and a manifest [`ContentType`] via
/// [`coerce_payload`] + [`typed_entry`].
#[derive(Debug, Clone)]
pub struct TypedEntry {
    pub path: String,
    pub universe_key: String,
    pub entry_type: String,
    /// Typed fields coerced from the payload JSON using the manifest schema.
    pub fields: BTreeMap<String, TypedValue>,
    pub body: String,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a JSON payload against a manifest [`ContentType`] schema.
///
/// Returns `Err(PayloadError)` with a dot-notation field path on the first
/// validation failure.
///
/// Rules:
/// - Required fields must be present and non-null.
/// - `enum` fields must contain one of the declared values (or be absent/null).
/// - `date` fields must be valid ISO-8601 date strings (or absent/null).
/// - `string` / `ref` fields must be strings (or absent/null).
/// - `number` fields must be numbers (or absent/null).
/// - `boolean` fields must be booleans (or absent/null).
/// - `ref_list` fields must be arrays of strings (or absent/null).
///
/// Fields not declared in the schema are ignored (forward-compat).
pub fn validate_payload(ct: &ContentType, payload: &JsonValue) -> Result<(), PayloadError> {
    for (field_name, field_def) in &ct.schema {
        let value = payload.get(field_name);

        if field_def.required {
            match value {
                None | Some(JsonValue::Null) => {
                    return Err(PayloadError {
                        path: field_name.clone(),
                        message: "required field is missing".to_string(),
                    });
                }
                _ => {}
            }
        }

        // Only validate non-null values; absent/null are allowed for optional fields.
        let v = match value {
            Some(v) if !v.is_null() => v,
            _ => continue,
        };

        match field_def.field_type {
            FieldType::String | FieldType::Ref => {
                if !v.is_string() {
                    return Err(PayloadError {
                        path: field_name.clone(),
                        message: "field must be a string".to_string(),
                    });
                }
            }
            FieldType::Number => {
                if !v.is_number() {
                    return Err(PayloadError {
                        path: field_name.clone(),
                        message: "field must be a number".to_string(),
                    });
                }
            }
            FieldType::Boolean => {
                if !v.is_boolean() {
                    return Err(PayloadError {
                        path: field_name.clone(),
                        message: "field must be a boolean".to_string(),
                    });
                }
            }
            FieldType::Enum => {
                let s = v.as_str().ok_or_else(|| PayloadError {
                    path: field_name.clone(),
                    message: "enum field must be a string".to_string(),
                })?;
                if !field_def.values.is_empty() && !field_def.values.iter().any(|val| val == s) {
                    return Err(PayloadError {
                        path: field_name.clone(),
                        message: format!(
                            "value '{}' not in allowed values: [{}]",
                            s,
                            field_def.values.join(", ")
                        ),
                    });
                }
            }
            FieldType::Date => {
                let s = v.as_str().ok_or_else(|| PayloadError {
                    path: field_name.clone(),
                    message: "date field must be a string".to_string(),
                })?;
                parse_date(s).map_err(|_| PayloadError {
                    path: field_name.clone(),
                    message: "date field must be a valid ISO-8601 date or datetime".to_string(),
                })?;
            }
            FieldType::RefList => {
                let arr = v.as_array().ok_or_else(|| PayloadError {
                    path: field_name.clone(),
                    message: "ref_list field must be an array".to_string(),
                })?;
                for (i, item) in arr.iter().enumerate() {
                    if !item.is_string() {
                        return Err(PayloadError {
                            path: format!("{}[{}]", field_name, i),
                            message: "ref_list items must be strings".to_string(),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Type coercion
// ---------------------------------------------------------------------------

/// Coerce all JSON fields in `payload` to typed Rust values using the manifest
/// schema.  Fields not declared in the schema are coerced by their JSON type.
pub fn coerce_payload(ct: &ContentType, payload: &JsonValue) -> BTreeMap<String, TypedValue> {
    let mut out = BTreeMap::new();
    let obj = match payload.as_object() {
        Some(o) => o,
        None => return out,
    };
    for (field_name, value) in obj {
        let typed = if let Some(def) = ct.schema.get(field_name) {
            coerce_typed(&def.field_type, value)
        } else {
            coerce_untyped(value)
        };
        out.insert(field_name.clone(), typed);
    }
    out
}

fn coerce_typed(field_type: &FieldType, value: &JsonValue) -> TypedValue {
    match field_type {
        FieldType::Date => {
            if let Some(s) = value.as_str()
                && let Ok(dt) = parse_date(s)
            {
                return TypedValue::Date(dt);
            }
            TypedValue::Null
        }
        FieldType::Number => value
            .as_f64()
            .map(TypedValue::Number)
            .unwrap_or(TypedValue::Null),
        FieldType::Boolean => value
            .as_bool()
            .map(TypedValue::Boolean)
            .unwrap_or(TypedValue::Null),
        FieldType::RefList => {
            if let Some(arr) = value.as_array() {
                let strings: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                TypedValue::StringArray(strings)
            } else {
                TypedValue::Null
            }
        }
        _ => coerce_untyped(value),
    }
}

fn coerce_untyped(value: &JsonValue) -> TypedValue {
    match value {
        JsonValue::Null => TypedValue::Null,
        JsonValue::Bool(b) => TypedValue::Boolean(*b),
        JsonValue::Number(n) => TypedValue::Number(n.as_f64().unwrap_or(0.0)),
        JsonValue::String(s) => TypedValue::String(s.clone()),
        JsonValue::Array(arr) => TypedValue::StringArray(
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
        ),
        JsonValue::Object(_) => TypedValue::Null,
    }
}

/// Build a [`TypedEntry`] from raw entry data and the matching manifest [`ContentType`].
pub fn typed_entry(
    path: &str,
    universe_key: &str,
    entry_type: &str,
    payload_json: &JsonValue,
    body: &str,
    ct: &ContentType,
) -> TypedEntry {
    TypedEntry {
        path: path.to_string(),
        universe_key: universe_key.to_string(),
        entry_type: entry_type.to_string(),
        fields: coerce_payload(ct, payload_json),
        body: body.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn parse_date(s: &str) -> Result<DateTime<Utc>, ()> {
    // Try full RFC3339/ISO-8601 datetime
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Ok(dt);
    }
    // Try date-only YYYY-MM-DD
    if let Ok(dt) = format!("{}T00:00:00Z", s).parse::<DateTime<Utc>>() {
        return Ok(dt);
    }
    Err(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ContentType, FieldDef, FieldType, Presentation};
    use serde_json::json;

    fn make_ct_with_fields(fields: &[(&str, FieldType, bool, Vec<String>)]) -> ContentType {
        let mut schema = BTreeMap::new();
        for (name, ft, required, values) in fields {
            schema.insert(
                name.to_string(),
                FieldDef {
                    field_type: ft.clone(),
                    required: *required,
                    values: values.clone(),
                    semantic: None,
                    target: if matches!(ft, FieldType::Ref | FieldType::RefList) {
                        Some("other".to_string())
                    } else {
                        None
                    },
                },
            );
        }
        ContentType {
            name: "test".to_string(),
            schema,
            presentation: Presentation::default(),
            indexes: vec![],
        }
    }

    // --- validate_payload: required fields ---

    #[test]
    fn test_required_field_missing_rejected() {
        let ct = make_ct_with_fields(&[("title", FieldType::String, true, vec![])]);
        let err = validate_payload(&ct, &json!({})).unwrap_err();
        assert_eq!(err.path, "title");
        assert!(err.message.contains("required"));
    }

    #[test]
    fn test_required_field_null_rejected() {
        let ct = make_ct_with_fields(&[("title", FieldType::String, true, vec![])]);
        let err = validate_payload(&ct, &json!({"title": null})).unwrap_err();
        assert_eq!(err.path, "title");
    }

    #[test]
    fn test_optional_field_absent_accepted() {
        let ct = make_ct_with_fields(&[("title", FieldType::String, false, vec![])]);
        assert!(validate_payload(&ct, &json!({})).is_ok());
    }

    #[test]
    fn test_optional_field_null_accepted() {
        let ct = make_ct_with_fields(&[("title", FieldType::String, false, vec![])]);
        assert!(validate_payload(&ct, &json!({"title": null})).is_ok());
    }

    // --- validate_payload: enum ---

    #[test]
    fn test_valid_enum_accepted() {
        let ct = make_ct_with_fields(&[(
            "status",
            FieldType::Enum,
            false,
            vec!["todo".into(), "done".into()],
        )]);
        assert!(validate_payload(&ct, &json!({"status": "todo"})).is_ok());
        assert!(validate_payload(&ct, &json!({"status": "done"})).is_ok());
    }

    #[test]
    fn test_invalid_enum_rejected_with_field_path() {
        let ct = make_ct_with_fields(&[(
            "status",
            FieldType::Enum,
            false,
            vec!["todo".into(), "done".into()],
        )]);
        let err = validate_payload(&ct, &json!({"status": "invalid"})).unwrap_err();
        assert_eq!(err.path, "status");
        assert!(err.to_string().contains("status"), "error: {err}");
    }

    #[test]
    fn test_enum_non_string_rejected() {
        let ct = make_ct_with_fields(&[("status", FieldType::Enum, false, vec!["todo".into()])]);
        let err = validate_payload(&ct, &json!({"status": 42})).unwrap_err();
        assert_eq!(err.path, "status");
    }

    // --- validate_payload: date ---

    #[test]
    fn test_valid_iso_datetime_accepted() {
        let ct = make_ct_with_fields(&[("due_at", FieldType::Date, false, vec![])]);
        assert!(validate_payload(&ct, &json!({"due_at": "2026-04-30T12:00:00Z"})).is_ok());
    }

    #[test]
    fn test_valid_date_only_accepted() {
        let ct = make_ct_with_fields(&[("due_at", FieldType::Date, false, vec![])]);
        assert!(validate_payload(&ct, &json!({"due_at": "2026-04-30"})).is_ok());
    }

    #[test]
    fn test_invalid_date_rejected_with_field_path() {
        let ct = make_ct_with_fields(&[("due_at", FieldType::Date, false, vec![])]);
        let err = validate_payload(&ct, &json!({"due_at": "not-a-date"})).unwrap_err();
        assert_eq!(err.path, "due_at");
        assert!(err.to_string().contains("due_at"), "error: {err}");
    }

    // --- validate_payload: ref_list ---

    #[test]
    fn test_valid_ref_list_accepted() {
        let ct = make_ct_with_fields(&[("attendees", FieldType::RefList, false, vec![])]);
        assert!(validate_payload(&ct, &json!({"attendees": ["alice", "bob"]})).is_ok());
    }

    #[test]
    fn test_ref_list_non_array_rejected() {
        let ct = make_ct_with_fields(&[("attendees", FieldType::RefList, false, vec![])]);
        let err = validate_payload(&ct, &json!({"attendees": "alice"})).unwrap_err();
        assert_eq!(err.path, "attendees");
    }

    #[test]
    fn test_ref_list_non_string_item_rejected_with_index() {
        let ct = make_ct_with_fields(&[("attendees", FieldType::RefList, false, vec![])]);
        let err = validate_payload(&ct, &json!({"attendees": ["alice", 42]})).unwrap_err();
        assert_eq!(err.path, "attendees[1]");
    }

    #[test]
    fn test_unknown_fields_ignored() {
        let ct = make_ct_with_fields(&[("title", FieldType::String, true, vec![])]);
        let payload = json!({"title": "hello", "extra_field": "ignored"});
        assert!(validate_payload(&ct, &payload).is_ok());
    }

    // --- coerce_payload ---

    #[test]
    fn test_date_field_coerced_to_datetime() {
        let ct = make_ct_with_fields(&[("due_at", FieldType::Date, false, vec![])]);
        let payload = json!({"due_at": "2026-04-30T12:00:00Z"});
        let fields = coerce_payload(&ct, &payload);
        assert!(matches!(fields["due_at"], TypedValue::Date(_)));
    }

    #[test]
    fn test_date_only_field_coerced_to_datetime() {
        let ct = make_ct_with_fields(&[("due_at", FieldType::Date, false, vec![])]);
        let payload = json!({"due_at": "2026-04-30"});
        let fields = coerce_payload(&ct, &payload);
        assert!(matches!(fields["due_at"], TypedValue::Date(_)));
    }

    #[test]
    fn test_number_field_coerced() {
        let ct = make_ct_with_fields(&[("count", FieldType::Number, false, vec![])]);
        let payload = json!({"count": 42});
        let fields = coerce_payload(&ct, &payload);
        assert_eq!(fields["count"], TypedValue::Number(42.0));
    }

    #[test]
    fn test_boolean_field_coerced() {
        let ct = make_ct_with_fields(&[("done", FieldType::Boolean, false, vec![])]);
        let payload = json!({"done": true});
        let fields = coerce_payload(&ct, &payload);
        assert_eq!(fields["done"], TypedValue::Boolean(true));
    }

    #[test]
    fn test_ref_list_field_coerced_to_string_array() {
        let ct = make_ct_with_fields(&[("attendees", FieldType::RefList, false, vec![])]);
        let payload = json!({"attendees": ["alice", "bob"]});
        let fields = coerce_payload(&ct, &payload);
        assert_eq!(
            fields["attendees"],
            TypedValue::StringArray(vec!["alice".into(), "bob".into()])
        );
    }

    #[test]
    fn test_untyped_string_coerced() {
        let ct = make_ct_with_fields(&[("title", FieldType::String, false, vec![])]);
        let payload = json!({"title": "hello", "untyped": "world"});
        let fields = coerce_payload(&ct, &payload);
        assert_eq!(fields["title"], TypedValue::String("hello".into()));
        assert_eq!(fields["untyped"], TypedValue::String("world".into()));
    }

    // --- typed_entry ---

    #[test]
    fn test_typed_entry_fields_coerced() {
        let ct = make_ct_with_fields(&[
            ("title", FieldType::String, true, vec![]),
            (
                "status",
                FieldType::Enum,
                false,
                vec!["todo".into(), "done".into()],
            ),
            ("due_at", FieldType::Date, false, vec![]),
        ]);
        let payload = json!({
            "title": "My Task",
            "status": "todo",
            "due_at": "2026-05-01"
        });
        let entry = typed_entry("tasks/1.md", "myuniverse", "tarefa", &payload, "body", &ct);
        assert_eq!(entry.path, "tasks/1.md");
        assert_eq!(entry.universe_key, "myuniverse");
        assert_eq!(entry.entry_type, "tarefa");
        assert_eq!(entry.fields["title"], TypedValue::String("My Task".into()));
        assert_eq!(entry.fields["status"], TypedValue::String("todo".into()));
        assert!(matches!(entry.fields["due_at"], TypedValue::Date(_)));
    }
}
