//! Content section parser for structured markdown
//!
//! Parses markdown body content to extract structured sections from headers.
//! Used for user stories (AS/I NEED/TO) and tasks (GIVEN/WHEN/THEN).

use std::collections::HashMap;

/// Specification for a content section to extract
#[derive(Debug, Clone)]
pub struct SectionSpec {
    /// Header text to match (case-insensitive, e.g., "As", "Given")
    pub header: String,
    /// Field name for the extracted content (e.g., "user", "system")
    pub field: String,
    /// Whether this section is required
    pub required: bool,
}

impl SectionSpec {
    /// Create a new required section spec
    pub fn required(header: &str, field: &str) -> Self {
        Self {
            header: header.to_string(),
            field: field.to_string(),
            required: true,
        }
    }

    /// Create a new optional section spec
    pub fn optional(header: &str, field: &str) -> Self {
        Self {
            header: header.to_string(),
            field: field.to_string(),
            required: false,
        }
    }
}

/// Parsed content from markdown body
#[derive(Debug, Clone, Default)]
pub struct ParsedContent {
    /// Title from first `# ` heading
    pub title: Option<String>,
    /// Extracted sections by field name
    pub sections: HashMap<String, String>,
    /// Full body text
    pub body: String,
}

impl ParsedContent {
    /// Parse markdown content extracting sections based on specs
    pub fn parse(body: &str, specs: &[SectionSpec]) -> Self {
        let mut result = ParsedContent {
            body: body.to_string(),
            ..Default::default()
        };

        let lines: Vec<&str> = body.lines().collect();
        let mut current_field: Option<String> = None;
        let mut section_content = String::new();

        for line in lines {
            // Check for title (# heading)
            if let Some(title) = line.strip_prefix("# ")
                && result.title.is_none()
            {
                result.title = Some(title.trim().to_string());
                continue;
            }

            // Check for section headers (## heading)
            if let Some(header_text) = line.strip_prefix("## ") {
                // Save previous section if any
                if let Some(ref field) = current_field {
                    let content = section_content.trim().to_string();
                    if !content.is_empty() {
                        result.sections.insert(field.clone(), content);
                    }
                }

                let header = header_text.trim();
                let normalized = header.to_lowercase();

                // Check if this matches any spec
                current_field = None;
                section_content.clear();

                for spec in specs {
                    if spec.header.to_lowercase() == normalized {
                        current_field = Some(spec.field.clone());
                        break;
                    }
                }
                continue;
            }

            // Accumulate content for current section
            if current_field.is_some() {
                if !section_content.is_empty() {
                    section_content.push('\n');
                }
                section_content.push_str(line);
            }
        }

        // Save final section
        if let Some(ref field) = current_field {
            let content = section_content.trim().to_string();
            if !content.is_empty() {
                result.sections.insert(field.clone(), content);
            }
        }

        result
    }

    /// Validate that required sections are present
    pub fn validate(&self, specs: &[SectionSpec]) -> Vec<String> {
        let mut missing = Vec::new();

        for spec in specs {
            if spec.required && !self.sections.contains_key(&spec.field) {
                missing.push(format!("Missing required section: ## {}", spec.header));
            }
        }

        missing
    }

    /// Get a section by field name
    pub fn get(&self, field: &str) -> Option<&str> {
        self.sections.get(field).map(|s| s.as_str())
    }
}

/// Predefined section specs for user stories
pub fn user_story_specs() -> Vec<SectionSpec> {
    vec![
        SectionSpec::required("As", "user"),
        SectionSpec::required("I Need", "feature"),
        SectionSpec::required("To", "value"),
    ]
}

/// Predefined section specs for tasks with acceptance criteria
pub fn task_specs() -> Vec<SectionSpec> {
    vec![
        SectionSpec::required("Given", "system"),
        SectionSpec::required("When", "action"),
        SectionSpec::required("Then", "outcome"),
    ]
}

/// Get section specs for a content type
pub fn specs_for_type(content_type: &str) -> Option<Vec<SectionSpec>> {
    match content_type {
        "user-story" => Some(user_story_specs()),
        "task" => Some(task_specs()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_user_story() {
        let content = r#"# User Agents

## As
a developer

## I Need
to add custom agents

## To
extend the agent registry
"#;

        let specs = user_story_specs();
        let parsed = ParsedContent::parse(content, &specs);

        assert_eq!(parsed.title, Some("User Agents".to_string()));
        assert_eq!(parsed.get("user"), Some("a developer"));
        assert_eq!(parsed.get("feature"), Some("to add custom agents"));
        assert_eq!(parsed.get("value"), Some("extend the agent registry"));
    }

    #[test]
    fn test_parse_task() {
        let content = r#"# Create user agent

## Given
`user/agents/` directory exists

## When
user adds `user/agents/my-agent.md`

## Then
agent appears in `co agents` output
"#;

        let specs = task_specs();
        let parsed = ParsedContent::parse(content, &specs);

        assert_eq!(parsed.title, Some("Create user agent".to_string()));
        assert_eq!(
            parsed.get("system"),
            Some("`user/agents/` directory exists")
        );
        assert_eq!(
            parsed.get("action"),
            Some("user adds `user/agents/my-agent.md`")
        );
        assert_eq!(
            parsed.get("outcome"),
            Some("agent appears in `co agents` output")
        );
    }

    #[test]
    fn test_validate_missing_sections() {
        let content = r#"# Incomplete Story

## As
a user
"#;

        let specs = user_story_specs();
        let parsed = ParsedContent::parse(content, &specs);
        let errors = parsed.validate(&specs);

        assert_eq!(errors.len(), 2);
        assert!(errors.iter().any(|e| e.contains("I Need")));
        assert!(errors.iter().any(|e| e.contains("To")));
    }

    #[test]
    fn test_case_insensitive_headers() {
        let content = r#"# Test

## AS
uppercase user

## i need
lowercase feature

## TO
mixed value
"#;

        let specs = user_story_specs();
        let parsed = ParsedContent::parse(content, &specs);

        assert_eq!(parsed.get("user"), Some("uppercase user"));
        assert_eq!(parsed.get("feature"), Some("lowercase feature"));
        assert_eq!(parsed.get("value"), Some("mixed value"));
    }

    #[test]
    fn test_multiline_sections() {
        let content = r#"# Complex Task

## Given
- precondition 1
- precondition 2
- precondition 3

## When
user performs a complex action
with multiple steps

## Then
expected outcome is achieved
"#;

        let specs = task_specs();
        let parsed = ParsedContent::parse(content, &specs);

        let system = parsed.get("system").unwrap();
        assert!(system.contains("precondition 1"));
        assert!(system.contains("precondition 2"));
        assert!(system.contains("precondition 3"));

        let action = parsed.get("action").unwrap();
        assert!(action.contains("complex action"));
        assert!(action.contains("multiple steps"));
    }

    #[test]
    fn test_unknown_sections_ignored() {
        let content = r#"# Story

## As
a user

## Background
some context that should be ignored

## I Need
a feature

## To
get value
"#;

        let specs = user_story_specs();
        let parsed = ParsedContent::parse(content, &specs);

        assert_eq!(parsed.sections.len(), 3);
        assert!(!parsed.sections.contains_key("background"));
    }

    #[test]
    fn test_specs_for_type() {
        assert!(specs_for_type("user-story").is_some());
        assert!(specs_for_type("task").is_some());
        assert!(specs_for_type("agent").is_none());
        assert!(specs_for_type("unknown").is_none());
    }
}
