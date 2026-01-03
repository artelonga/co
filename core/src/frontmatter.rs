//! YAML frontmatter extraction
//!
//! Extract structured data from Markdown files with YAML frontmatter.
//!
//! # Flexibility
//!
//! Frontmatter is generic over the data type T. Extended types can define
//! their own structs with additional fields. Use `#[serde(flatten)]` to
//! extend BaseFrontmatter, or define completely custom structures.
//!
//! Core tests only verify universal fields (type, id). Extended types
//! may have context-specific required/optional fields.
//!
//! ## Example: Extended Frontmatter
//!
//! ```ignore
//! #[derive(Deserialize)]
//! struct TaskFrontmatter {
//!     #[serde(flatten)]
//!     base: BaseFrontmatter,
//!     status: String,           // Required for tasks
//!     priority: Option<String>, // Optional
//! }
//! ```

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

/// Default schema version for frontmatter
fn default_schema_version() -> u32 {
    1
}

/// Base frontmatter structure with universal fields.
///
/// Only includes fields common to ALL content types.
/// Extended types should define their own structs for type-specific fields.
///
/// - `type`: Content type identifier (required)
/// - `id`: Unique identifier (required)
/// - `schema_version`: For migration support (defaults to 1)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BaseFrontmatter {
    /// Content type (definition, task, project, context, language, etc.)
    #[serde(rename = "type")]
    pub content_type: String,

    /// Unique identifier for this content
    pub id: String,

    /// Schema version for migration support
    /// Defaults to 1 for backward compatibility
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
}

/// Errors that can occur during frontmatter parsing
#[derive(Error, Debug)]
pub enum FrontmatterError {
    #[error("No frontmatter found (file must start with ---)")]
    NotFound,

    #[error("Invalid frontmatter format: {0}")]
    InvalidFormat(String),

    #[error("YAML parse error: {0}")]
    YamlError(#[from] serde_yaml::Error),
}

/// Parsed frontmatter with optional body
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter<T> {
    /// The parsed YAML data
    pub data: T,

    /// Byte offset where body content starts
    pub body_offset: usize,
}

impl<T: DeserializeOwned> Frontmatter<T> {
    /// Parse frontmatter from bytes
    ///
    /// This is efficient - it finds the YAML section without
    /// loading the entire file body into memory.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FrontmatterError> {
        // Check for frontmatter delimiter
        if !bytes.starts_with(b"---\n") && !bytes.starts_with(b"---\r\n") {
            return Err(FrontmatterError::NotFound);
        }

        // Find the end delimiter
        let start = if bytes.starts_with(b"---\r\n") { 5 } else { 4 };

        let end = find_end_delimiter(&bytes[start..])
            .ok_or_else(|| FrontmatterError::InvalidFormat("No closing --- found".into()))?;

        let yaml_bytes = &bytes[start..start + end];
        let yaml_str = std::str::from_utf8(yaml_bytes)
            .map_err(|e| FrontmatterError::InvalidFormat(e.to_string()))?;

        let data: T = serde_yaml::from_str(yaml_str)?;

        // Body starts after the closing delimiter
        let body_offset = start + end + 4; // 4 for "\n---\n" or similar

        Ok(Frontmatter { data, body_offset })
    }

    /// Parse frontmatter from a string
    pub fn parse(content: &str) -> Result<Self, FrontmatterError> {
        Self::from_bytes(content.as_bytes())
    }
}

/// Find the position of the end delimiter
fn find_end_delimiter(bytes: &[u8]) -> Option<usize> {
    let patterns: &[&[u8]] = &[b"\n---\n", b"\n---\r\n", b"\r\n---\r\n", b"\r\n---\n"];

    for pattern in patterns {
        if let Some(pos) = bytes
            .windows(pattern.len())
            .position(|window| window == *pattern)
        {
            return Some(pos);
        }
    }

    None
}

/// Extract just the frontmatter portion as raw YAML
pub fn extract_yaml(content: &str) -> Option<&str> {
    if !content.starts_with("---") {
        return None;
    }

    let start = content.find('\n')? + 1;
    let rest = &content[start..];
    let end = rest.find("\n---")?;

    Some(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestFrontmatter {
        title: String,
        status: String,
    }

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
title: Test Task
status: todo
---

# Body content here
"#;

        let fm: Frontmatter<TestFrontmatter> = Frontmatter::parse(content).unwrap();
        assert_eq!(fm.data.title, "Test Task");
        assert_eq!(fm.data.status, "todo");
    }

    #[test]
    fn test_no_frontmatter() {
        let content = "# Just a heading\n\nSome content";
        let result: Result<Frontmatter<TestFrontmatter>, _> = Frontmatter::parse(content);
        assert!(matches!(result, Err(FrontmatterError::NotFound)));
    }

    #[test]
    fn test_schema_version_defaults_to_1() {
        // v0.1.0 files without schema_version should default to 1
        let content = r#"---
type: definition
id: hello
---

A greeting.
"#;

        let fm: Frontmatter<BaseFrontmatter> = Frontmatter::parse(content).unwrap();
        assert_eq!(fm.data.schema_version, 1);
    }

    #[test]
    fn test_schema_version_explicit() {
        // v0.2.0 files can specify schema_version: 2
        let content = r#"---
type: definition
id: hello
schema_version: 2
---

A greeting.
"#;

        let fm: Frontmatter<BaseFrontmatter> = Frontmatter::parse(content).unwrap();
        assert_eq!(fm.data.schema_version, 2);
    }

    #[test]
    fn test_extended_frontmatter_with_extra_fields() {
        // Extended types can have additional fields beyond base
        #[derive(Debug, Deserialize, PartialEq)]
        struct ExtendedFrontmatter {
            #[serde(rename = "type")]
            content_type: String,
            id: String,
            // Extended fields - context-specific
            status: String,
            priority: Option<String>,
            #[serde(default)]
            tags: Vec<String>,
        }

        let content = r#"---
type: task
id: my-task
status: active
priority: high
tags:
  - urgent
  - backend
---

Task description here.
"#;

        let fm: Frontmatter<ExtendedFrontmatter> = Frontmatter::parse(content).unwrap();
        assert_eq!(fm.data.content_type, "task");
        assert_eq!(fm.data.status, "active");
        assert_eq!(fm.data.priority, Some("high".to_string()));
        assert_eq!(fm.data.tags, vec!["urgent", "backend"]);
    }

    #[test]
    fn test_base_frontmatter_ignores_extra_fields() {
        // BaseFrontmatter only parses universal fields, ignores the rest
        let content = r#"---
type: task
id: my-task
status: active
custom_field: ignored
---

Body.
"#;

        let fm: Frontmatter<BaseFrontmatter> = Frontmatter::parse(content).unwrap();
        assert_eq!(fm.data.content_type, "task");
        assert_eq!(fm.data.id, "my-task");
        // Extra fields are simply not captured, no error
    }
}
