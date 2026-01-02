//! YAML frontmatter extraction
//!
//! Extract structured data from Markdown files with YAML frontmatter.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use thiserror::Error;

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
}
