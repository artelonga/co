//! Mapping between GitHub issues and CO content files

use super::types::GithubIssue;

/// Represents a mapped work item ready to be written as a markdown file
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedWorkItem {
    /// Content type (task, user-story, epic, etc.)
    pub content_type: String,
    /// Item ID (e.g., "task-35")
    pub id: String,
    /// GitHub issue number
    pub github_number: u64,
    /// Status (todo, in-progress, done)
    pub status: String,
    /// Priority (optional)
    pub priority: Option<String>,
    /// Assignee username (optional)
    pub assignee: Option<String>,
    /// Title (from issue title)
    pub title: String,
    /// Body content (from issue body)
    pub body: String,
}

impl MappedWorkItem {
    /// Convert a GitHub issue to a mapped work item
    pub fn from_github_issue(issue: &GithubIssue) -> Self {
        let content_type = issue.content_type().unwrap_or_else(|| "task".to_string());
        let id = format!("{}-{}", content_type, issue.number);

        Self {
            content_type,
            id,
            github_number: issue.number,
            status: issue.status().to_string(),
            priority: issue.priority(),
            assignee: issue.first_assignee().map(|s| s.to_string()),
            title: issue.title.clone(),
            body: issue.body.clone(),
        }
    }

    /// Generate the file path for this work item
    pub fn file_path(&self, space: &str) -> String {
        let type_dir = pluralize_type(&self.content_type);
        format!("{}/{}/{}.md", space, type_dir, self.id)
    }

    /// Generate the markdown content with frontmatter
    pub fn to_markdown(&self, space: &str, language: &str) -> String {
        let mut frontmatter = format!(
            r#"---
schema_version: 2
language: {}
space: {}
type: {}
id: {}
github: {}
status: {}"#,
            language, space, self.content_type, self.id, self.github_number, self.status
        );

        if let Some(ref priority) = self.priority {
            frontmatter.push_str(&format!("\npriority: {}", priority));
        }

        if let Some(ref assignee) = self.assignee {
            frontmatter.push_str(&format!("\nassignee: {}", assignee));
        }

        frontmatter.push_str("\n---\n\n");

        // Add title and body
        let body = if self.body.is_empty() {
            String::new()
        } else {
            format!("\n{}", self.body)
        };

        format!("{}# {}{}\n", frontmatter, self.title, body)
    }
}

/// Pluralize a content type for directory names
fn pluralize_type(content_type: &str) -> String {
    match content_type {
        "user-story" => "user-stories".to_string(),
        "use-case" => "use-cases".to_string(),
        t if t.ends_with('s') => t.to_string(),
        t => format!("{}s", t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::types::{GithubLabel, GithubUser};

    fn make_issue(number: u64, labels: Vec<&str>) -> GithubIssue {
        GithubIssue {
            number,
            title: format!("Test Issue {}", number),
            body: "Issue body content.".into(),
            state: "open".into(),
            labels: labels
                .into_iter()
                .map(|name| GithubLabel { name: name.into() })
                .collect(),
            assignees: vec![GithubUser {
                login: "testuser".into(),
            }],
            url: String::new(),
        }
    }

    #[test]
    fn test_from_github_issue_task() {
        let issue = make_issue(35, vec!["type:task", "priority:high"]);
        let mapped = MappedWorkItem::from_github_issue(&issue);

        assert_eq!(mapped.content_type, "task");
        assert_eq!(mapped.id, "task-35");
        assert_eq!(mapped.github_number, 35);
        assert_eq!(mapped.status, "todo");
        assert_eq!(mapped.priority, Some("high".into()));
        assert_eq!(mapped.assignee, Some("testuser".into()));
        assert_eq!(mapped.title, "Test Issue 35");
    }

    #[test]
    fn test_from_github_issue_user_story() {
        let issue = make_issue(10, vec!["type:user-story"]);
        let mapped = MappedWorkItem::from_github_issue(&issue);

        assert_eq!(mapped.content_type, "user-story");
        assert_eq!(mapped.id, "user-story-10");
    }

    #[test]
    fn test_from_github_issue_no_type_defaults_to_task() {
        let issue = make_issue(99, vec!["priority:low"]);
        let mapped = MappedWorkItem::from_github_issue(&issue);

        assert_eq!(mapped.content_type, "task");
        assert_eq!(mapped.id, "task-99");
    }

    #[test]
    fn test_file_path() {
        let issue = make_issue(35, vec!["type:task"]);
        let mapped = MappedWorkItem::from_github_issue(&issue);

        assert_eq!(mapped.file_path("work"), "work/tasks/task-35.md");
    }

    #[test]
    fn test_file_path_user_story() {
        let issue = make_issue(10, vec!["type:user-story"]);
        let mapped = MappedWorkItem::from_github_issue(&issue);

        assert_eq!(
            mapped.file_path("work"),
            "work/user-stories/user-story-10.md"
        );
    }

    #[test]
    fn test_to_markdown() {
        let issue = make_issue(35, vec!["type:task", "priority:high"]);
        let mapped = MappedWorkItem::from_github_issue(&issue);
        let markdown = mapped.to_markdown("work", "en");

        assert!(markdown.contains("schema_version: 2"));
        assert!(markdown.contains("language: en"));
        assert!(markdown.contains("space: work"));
        assert!(markdown.contains("type: task"));
        assert!(markdown.contains("id: task-35"));
        assert!(markdown.contains("github: 35"));
        assert!(markdown.contains("status: todo"));
        assert!(markdown.contains("priority: high"));
        assert!(markdown.contains("assignee: testuser"));
        assert!(markdown.contains("# Test Issue 35"));
        assert!(markdown.contains("Issue body content."));
    }

    #[test]
    fn test_to_markdown_minimal() {
        let issue = GithubIssue {
            number: 1,
            title: "Simple Issue".into(),
            body: String::new(),
            state: "closed".into(),
            labels: vec![],
            assignees: vec![],
            url: String::new(),
        };
        let mapped = MappedWorkItem::from_github_issue(&issue);
        let markdown = mapped.to_markdown("work", "en");

        assert!(markdown.contains("type: task")); // default
        assert!(markdown.contains("status: done")); // closed
        assert!(!markdown.contains("priority:")); // no priority
        assert!(!markdown.contains("assignee:")); // no assignee
        assert!(markdown.contains("# Simple Issue"));
    }

    #[test]
    fn test_pluralize_type() {
        assert_eq!(pluralize_type("task"), "tasks");
        assert_eq!(pluralize_type("epic"), "epics");
        assert_eq!(pluralize_type("user-story"), "user-stories");
        assert_eq!(pluralize_type("use-case"), "use-cases");
        assert_eq!(pluralize_type("tasks"), "tasks"); // already plural
    }
}
