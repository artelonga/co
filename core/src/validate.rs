//! Content validation
//!
//! Validates frontmatter fields and references for content integrity.

use crate::content::{ParsedContent, specs_for_type};
use crate::feature::FeatureRegistry;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Severity level for validation issues
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// Must be fixed
    Error,
    /// Should be addressed
    Warning,
}

/// A validation issue found in a file
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// File path where the issue was found
    pub file: PathBuf,
    /// Severity level
    pub severity: Severity,
    /// Description of the issue
    pub message: String,
}

impl ValidationIssue {
    /// Create an error-level issue
    pub fn error(file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            severity: Severity::Error,
            message: message.into(),
        }
    }

    /// Create a warning-level issue
    pub fn warning(file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

/// Context for validation operations
#[derive(Debug, Default)]
pub struct ValidationContext {
    /// Root path of the repository
    pub root: PathBuf,
    /// Known content IDs (for link validation)
    pub known_ids: HashSet<String>,
    /// Feature registry for extensible content types
    pub feature_registry: Option<FeatureRegistry>,
}

/// Known built-in content types
pub const KNOWN_TYPES: &[&str] = &[
    "task",
    "definition",
    "project",
    "space",
    "language",
    "agent",
    "tool",
    "content",
    // Work item types (EPIC 13)
    "user-story",
    "epic",
    "release",
];

impl ValidationContext {
    /// Create a new validation context
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root_path = root.into();
        let feature_registry = Some(FeatureRegistry::discover(&root_path));
        Self {
            root: root_path,
            known_ids: HashSet::new(),
            feature_registry,
        }
    }

    /// Check if a content type is known
    ///
    /// Checks both built-in types and types registered via feature schemas.
    pub fn type_exists(&self, content_type: &str) -> bool {
        // Check built-in types first
        if KNOWN_TYPES.contains(&content_type) {
            return true;
        }

        // Check feature registry for custom types
        self.feature_registry
            .as_ref()
            .is_some_and(|r| r.has_content_type(content_type))
    }

    /// Check if a language is valid
    ///
    /// Known languages (english, portuguese, etc.) are always valid.
    /// Unknown languages require a corresponding directory to exist.
    pub fn language_exists(&self, language: &str) -> bool {
        // Known languages and their directory codes
        let known_languages = [
            ("english", "en"),
            ("en", "en"),
            ("portuguese", "pt"),
            ("pt", "pt"),
            ("guarani-mbya", "gun"),
            ("gun", "gun"),
            ("spanish", "es"),
            ("es", "es"),
            ("french", "fr"),
            ("fr", "fr"),
            ("german", "de"),
            ("de", "de"),
        ];

        // Check if it's a known language
        for (name, _code) in &known_languages {
            if language == *name {
                return true;
            }
        }

        // For unknown languages, check if directory exists
        self.root.join(language).is_dir()
    }

    /// Check if a space directory exists
    pub fn space_exists(&self, space: &str) -> bool {
        self.root.join(space).is_dir()
    }

    /// Deprecated: use space_exists instead
    #[deprecated(since = "0.13.1", note = "Use space_exists instead")]
    pub fn scope_exists(&self, scope: &str) -> bool {
        self.space_exists(scope)
    }
}

/// Frontmatter fields for validation
#[derive(Debug, Deserialize)]
struct ValidatableFrontmatter {
    #[serde(rename = "type")]
    content_type: Option<String>,
    #[allow(dead_code)]
    id: Option<String>,
    language: Option<String>,
    scope: Option<String>,
}

/// Validate a single content file
///
/// Returns a list of validation issues found.
pub fn validate_file(path: &Path, ctx: &ValidationContext) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Read file content
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            issues.push(ValidationIssue::error(
                path,
                format!("Cannot read file: {}", e),
            ));
            return issues;
        }
    };

    // Check for frontmatter
    if !content.starts_with("---") {
        issues.push(ValidationIssue::error(
            path,
            "No frontmatter found (file must start with ---)",
        ));
        return issues;
    }

    // Find end of frontmatter
    let end_idx = match content[3..].find("\n---") {
        Some(idx) => idx,
        None => {
            issues.push(ValidationIssue::error(
                path,
                "Invalid frontmatter format: no closing ---",
            ));
            return issues;
        }
    };

    let yaml_str = &content[4..end_idx + 3];

    // Parse frontmatter
    let frontmatter: ValidatableFrontmatter = match serde_yaml::from_str(yaml_str) {
        Ok(fm) => fm,
        Err(e) => {
            issues.push(ValidationIssue::error(
                path,
                format!("YAML parse error: {}", e),
            ));
            return issues;
        }
    };

    // Task 5.1.1: Check required field: language
    if frontmatter.language.is_none() {
        issues.push(ValidationIssue::error(
            path,
            "Missing required field: language",
        ));
    }

    // Task 5.1.2: Check language exists (has lexicon directory)
    if let Some(ref lang) = frontmatter.language
        && !ctx.language_exists(lang)
    {
        issues.push(ValidationIssue::error(
            path,
            format!("Unknown language: {} (no lexicon found)", lang),
        ));
    }

    // Task 5.1.3: Check space exists (if specified)
    if let Some(ref scope) = frontmatter.scope
        && !ctx.space_exists(scope)
    {
        issues.push(ValidationIssue::error(
            path,
            format!("Unknown space: {}", scope),
        ));
    }

    // Task 5.1.4: Check internal links [[reference]]
    let body_start = end_idx + 7; // Skip past "\n---\n"
    if body_start < content.len() {
        let body = &content[body_start..];
        let links = extract_internal_links(body);
        for link in links {
            if !ctx.known_ids.contains(&link) {
                issues.push(ValidationIssue::warning(
                    path,
                    format!("Broken link: {}", link),
                ));
            }
        }
    }

    // Task 5.1.5: Check type reference exists
    if let Some(ref content_type) = frontmatter.content_type {
        if !ctx.type_exists(content_type) {
            // Determine language directory for error message
            let lang_dir = frontmatter
                .language
                .as_ref()
                .map(|l| language_to_dir(l))
                .unwrap_or("en");
            issues.push(ValidationIssue::warning(
                path,
                format!(
                    "Unknown type: {} (not in {}/ lexicon)",
                    content_type, lang_dir
                ),
            ));
        }

        // Task 13.2.3: Validate required content sections for user-story and task types
        if let Some(specs) = specs_for_type(content_type) {
            let body_start = end_idx + 7; // Skip past "\n---\n"
            if body_start < content.len() {
                let body = &content[body_start..];
                let parsed = ParsedContent::parse(body, &specs);
                let missing = parsed.validate(&specs);

                for msg in missing {
                    issues.push(ValidationIssue::warning(path, msg));
                }
            }
        }
    }

    issues
}

/// Convert language name to directory name
fn language_to_dir(language: &str) -> &str {
    match language {
        "english" => "en",
        "portuguese" => "pt",
        "guarani-mbya" => "gun",
        other => other,
    }
}

/// Extract internal links from content body
///
/// Finds all `[[link]]` patterns and returns the link targets.
fn extract_internal_links(body: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = body;

    while let Some(start) = rest.find("[[") {
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("]]") {
            let link = after_start[..end].trim().to_string();
            if !link.is_empty() {
                links.push(link);
            }
            rest = &after_start[end + 2..];
        } else {
            break;
        }
    }

    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    fn create_context_with_dirs(dir: &TempDir, dirs: &[&str]) -> ValidationContext {
        for d in dirs {
            fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        ValidationContext::new(dir.path())
    }

    #[test]
    fn test_missing_language_field() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
---

Definition content here.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert_eq!(issues[0].message, "Missing required field: language");
    }

    #[test]
    fn test_valid_file_with_language() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
language: english
---

Definition content here.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert!(issues.is_empty());
    }

    #[test]
    fn test_no_frontmatter() {
        let dir = TempDir::new().unwrap();
        let ctx = ValidationContext::new(dir.path());
        let path = create_test_file(&dir, "test.md", "# Just a heading\n\nNo frontmatter here.");

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("No frontmatter found"));
    }

    #[test]
    fn test_invalid_yaml() {
        let dir = TempDir::new().unwrap();
        let ctx = ValidationContext::new(dir.path());
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: [unclosed
---

Content.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert!(issues[0].message.contains("YAML parse error"));
    }

    #[test]
    fn test_unclosed_frontmatter() {
        let dir = TempDir::new().unwrap();
        let ctx = ValidationContext::new(dir.path());
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: task
id: test
language: english

No closing delimiter."#,
        );

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("no closing ---"));
    }

    // Task 5.1.2: Language existence tests
    #[test]
    fn test_unknown_language() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en"]); // Only "en" exists
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
language: nonexistent
---

Definition content here.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert_eq!(
            issues[0].message,
            "Unknown language: nonexistent (no lexicon found)"
        );
    }

    #[test]
    fn test_language_exists_english() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
language: english
---

Content.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert!(issues.is_empty());
    }

    #[test]
    fn test_language_exists_portuguese() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["pt"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
language: portuguese
---

Content.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert!(issues.is_empty());
    }

    #[test]
    fn test_language_context_checks_directory() {
        let dir = TempDir::new().unwrap();
        // Create "en" and "pt" directories
        let ctx = create_context_with_dirs(&dir, &["en", "pt"]);

        // Known languages are always valid (no directory required)
        assert!(ctx.language_exists("english"));
        assert!(ctx.language_exists("portuguese"));
        assert!(ctx.language_exists("french")); // Known language, valid even without fr/

        // Unknown languages require a directory to exist
        assert!(!ctx.language_exists("klingon")); // Unknown, no directory
    }

    // Task 5.1.3: Scope existence tests
    #[test]
    fn test_unknown_scope() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en", "private"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
language: english
scope: nonexistent
---

Definition content here.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert_eq!(issues[0].message, "Unknown space: nonexistent");
    }

    #[test]
    fn test_valid_scope() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en", "private"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
language: english
scope: private
---

Content.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert!(issues.is_empty());
    }

    #[test]
    fn test_scope_context_checks_directory() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en", "private", "shared"]);

        assert!(ctx.space_exists("private"));
        assert!(ctx.space_exists("shared"));
        assert!(!ctx.space_exists("unknown"));
    }

    // Task 5.1.4: Internal link validation tests
    #[test]
    fn test_extract_internal_links() {
        let body = "Some text with [[link1]] and [[another-link]] in it.";
        let links = extract_internal_links(body);
        assert_eq!(links, vec!["link1", "another-link"]);
    }

    #[test]
    fn test_extract_internal_links_empty() {
        let body = "No links here.";
        let links = extract_internal_links(body);
        assert!(links.is_empty());
    }

    #[test]
    fn test_extract_internal_links_with_whitespace() {
        let body = "Link with [[ spaced ]] content.";
        let links = extract_internal_links(body);
        assert_eq!(links, vec!["spaced"]);
    }

    #[test]
    fn test_broken_link_warning() {
        let dir = TempDir::new().unwrap();
        let mut ctx = create_context_with_dirs(&dir, &["en"]);
        // Add some known IDs
        ctx.known_ids.insert("existing-def".to_string());

        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
language: english
---

This references [[other-def]] which does not exist.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(issues[0].message, "Broken link: other-def");
    }

    #[test]
    fn test_valid_link() {
        let dir = TempDir::new().unwrap();
        let mut ctx = create_context_with_dirs(&dir, &["en"]);
        ctx.known_ids.insert("existing-def".to_string());

        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
language: english
---

This references [[existing-def]] which exists.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert!(issues.is_empty());
    }

    #[test]
    fn test_multiple_broken_links() {
        let dir = TempDir::new().unwrap();
        let mut ctx = create_context_with_dirs(&dir, &["en"]);
        ctx.known_ids.insert("existing".to_string());

        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: my-def
language: english
---

See [[missing1]] and [[existing]] and [[missing2]].
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 2);
        assert!(issues.iter().any(|i| i.message == "Broken link: missing1"));
        assert!(issues.iter().any(|i| i.message == "Broken link: missing2"));
    }

    // Task 5.1.5: Type reference validation tests
    #[test]
    fn test_unknown_type_warning() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: custom-type
id: my-item
language: english
---

Content with unknown type.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(
            issues[0].message,
            "Unknown type: custom-type (not in en/ lexicon)"
        );
    }

    #[test]
    fn test_valid_type_task() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: task
id: my-task
language: english
---

# My Task

## Given
a valid task setup

## When
the task runs

## Then
it completes successfully
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert!(issues.is_empty());
    }

    #[test]
    fn test_valid_type_definition() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: definition
id: hello
language: english
---

A greeting.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert!(issues.is_empty());
    }

    #[test]
    fn test_unknown_type_with_portuguese() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["pt"]);
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: tipo-desconhecido
id: meu-item
language: portuguese
---

Conteúdo.
"#,
        );

        let issues = validate_file(&path, &ctx);

        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("not in pt/ lexicon"));
    }

    #[test]
    fn test_type_exists_method() {
        let dir = TempDir::new().unwrap();
        let ctx = ValidationContext::new(dir.path());

        assert!(ctx.type_exists("task"));
        assert!(ctx.type_exists("definition"));
        assert!(ctx.type_exists("project"));
        assert!(ctx.type_exists("space"));
        assert!(ctx.type_exists("language"));
        assert!(ctx.type_exists("agent"));
        assert!(ctx.type_exists("content"));
        assert!(!ctx.type_exists("unknown-type"));
    }

    // =========================================================================
    // Issue #35: Extensible Content Types Tests
    // =========================================================================

    #[test]
    fn test_custom_type_recognized_via_feature_registry() {
        let dir = TempDir::new().unwrap();

        // Create a custom "notes" feature with schema
        let notes_dir = dir.path().join("notes");
        fs::create_dir_all(&notes_dir).unwrap();
        fs::write(
            notes_dir.join("schema.yaml"),
            r#"name: notes
version: 1
content_types:
  - note
properties:
  status:
    kind: text
"#,
        )
        .unwrap();

        // Create a file with custom "note" type
        let en_dir = dir.path().join("en");
        fs::create_dir_all(en_dir.join("notes")).unwrap();
        let path = create_test_file(
            &dir,
            "en/notes/my-note.md",
            r#"---
type: note
id: my-note
language: english
status: draft
---

# My Note

Some content.
"#,
        );

        // Context with feature registry should recognize "note" type
        let ctx = create_context_with_dirs(&dir, &["en"]);
        let issues = validate_file(&path, &ctx);

        // Should NOT have "Unknown type: note" warning
        let type_warnings: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("Unknown type: note"))
            .collect();
        assert!(
            type_warnings.is_empty(),
            "Custom type 'note' should be recognized via FeatureRegistry, but got: {:?}",
            type_warnings
        );
    }

    #[test]
    fn test_builtin_work_types_recognized() {
        let dir = TempDir::new().unwrap();

        // Create work directory with built-in types
        let work_dir = dir.path().join("work");
        fs::create_dir_all(&work_dir).unwrap();
        fs::write(
            work_dir.join("schema.yaml"),
            r#"name: work
version: 1
content_types:
  - epic
  - user-story
  - task
  - use-case
properties:
  status:
    kind: text
    required: true
"#,
        )
        .unwrap();

        let en_dir = dir.path().join("en");
        fs::create_dir_all(en_dir.join("epics")).unwrap();

        // Test epic type
        let path = create_test_file(
            &dir,
            "en/epics/my-epic.md",
            r#"---
type: epic
id: my-epic
language: english
status: planned
---

# My Epic

Epic description.
"#,
        );

        let ctx = create_context_with_dirs(&dir, &["en"]);
        let issues = validate_file(&path, &ctx);

        // Should recognize "epic" as valid type
        let type_warnings: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("Unknown type"))
            .collect();
        assert!(
            type_warnings.is_empty(),
            "Built-in 'epic' type should be recognized, but got: {:?}",
            type_warnings
        );
    }

    #[test]
    fn test_truly_unknown_type_still_warns() {
        let dir = TempDir::new().unwrap();
        let ctx = create_context_with_dirs(&dir, &["en"]);

        // Type that doesn't exist in any schema
        let path = create_test_file(
            &dir,
            "test.md",
            r#"---
type: completely-made-up-type
id: test
language: english
---

Content.
"#,
        );

        let issues = validate_file(&path, &ctx);

        // Should warn about truly unknown type
        let type_warnings: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("Unknown type: completely-made-up-type"))
            .collect();
        assert_eq!(
            type_warnings.len(),
            1,
            "Should warn about truly unknown type"
        );
    }
}
