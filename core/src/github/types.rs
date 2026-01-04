//! GitHub data types for issue synchronization

use serde::{Deserialize, Serialize};

/// A GitHub issue label
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubLabel {
    /// Label name (e.g., "type:task", "priority:high")
    pub name: String,
}

/// A GitHub user (for assignees)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubUser {
    /// GitHub username
    pub login: String,
}

/// A GitHub issue fetched from the API
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubIssue {
    /// Issue number
    pub number: u64,
    /// Issue title
    pub title: String,
    /// Issue body (markdown)
    #[serde(default)]
    pub body: String,
    /// Issue state: "open" or "closed"
    pub state: String,
    /// Labels attached to the issue
    #[serde(default)]
    pub labels: Vec<GithubLabel>,
    /// Assigned users
    #[serde(default)]
    pub assignees: Vec<GithubUser>,
    /// URL to the issue
    #[serde(default)]
    pub url: String,
}

impl GithubIssue {
    /// Extract a typed label value (e.g., "type:task" -> Some("task"))
    pub fn get_label_value(&self, prefix: &str) -> Option<String> {
        let search = format!("{}:", prefix);
        self.labels
            .iter()
            .find(|l| l.name.starts_with(&search))
            .map(|l| l.name.strip_prefix(&search).unwrap_or("").to_string())
    }

    /// Get the content type from labels (type:task, type:user-story, etc.)
    pub fn content_type(&self) -> Option<String> {
        self.get_label_value("type")
    }

    /// Get priority from labels (priority:high, priority:low, etc.)
    pub fn priority(&self) -> Option<String> {
        self.get_label_value("priority")
    }

    /// Get the first assignee's login
    pub fn first_assignee(&self) -> Option<&str> {
        self.assignees.first().map(|u| u.login.as_str())
    }

    /// Convert state to CO status
    pub fn status(&self) -> &str {
        match self.state.as_str() {
            "closed" => "done",
            _ => "todo",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_issue_json() {
        let json = r#"{
            "number": 35,
            "title": "Test Issue",
            "body": "Issue description",
            "state": "open",
            "labels": [
                {"name": "type:task"},
                {"name": "priority:high"}
            ],
            "assignees": [
                {"login": "testuser"}
            ],
            "url": "https://github.com/org/repo/issues/35"
        }"#;

        let issue: GithubIssue = serde_json::from_str(json).unwrap();

        assert_eq!(issue.number, 35);
        assert_eq!(issue.title, "Test Issue");
        assert_eq!(issue.body, "Issue description");
        assert_eq!(issue.state, "open");
        assert_eq!(issue.labels.len(), 2);
        assert_eq!(issue.assignees.len(), 1);
    }

    #[test]
    fn test_get_label_value() {
        let issue = GithubIssue {
            number: 1,
            title: "Test".into(),
            body: String::new(),
            state: "open".into(),
            labels: vec![
                GithubLabel {
                    name: "type:task".into(),
                },
                GithubLabel {
                    name: "priority:high".into(),
                },
                GithubLabel {
                    name: "milestone:v1.0".into(),
                },
            ],
            assignees: vec![],
            url: String::new(),
        };

        assert_eq!(issue.get_label_value("type"), Some("task".into()));
        assert_eq!(issue.get_label_value("priority"), Some("high".into()));
        assert_eq!(issue.get_label_value("milestone"), Some("v1.0".into()));
        assert_eq!(issue.get_label_value("unknown"), None);
    }

    #[test]
    fn test_content_type_and_priority() {
        let issue = GithubIssue {
            number: 1,
            title: "Test".into(),
            body: String::new(),
            state: "open".into(),
            labels: vec![
                GithubLabel {
                    name: "type:user-story".into(),
                },
                GithubLabel {
                    name: "priority:low".into(),
                },
            ],
            assignees: vec![],
            url: String::new(),
        };

        assert_eq!(issue.content_type(), Some("user-story".into()));
        assert_eq!(issue.priority(), Some("low".into()));
    }

    #[test]
    fn test_status_mapping() {
        let open_issue = GithubIssue {
            number: 1,
            title: "Test".into(),
            body: String::new(),
            state: "open".into(),
            labels: vec![],
            assignees: vec![],
            url: String::new(),
        };
        assert_eq!(open_issue.status(), "todo");

        let closed_issue = GithubIssue {
            number: 2,
            title: "Test".into(),
            body: String::new(),
            state: "closed".into(),
            labels: vec![],
            assignees: vec![],
            url: String::new(),
        };
        assert_eq!(closed_issue.status(), "done");
    }

    #[test]
    fn test_first_assignee() {
        let issue = GithubIssue {
            number: 1,
            title: "Test".into(),
            body: String::new(),
            state: "open".into(),
            labels: vec![],
            assignees: vec![
                GithubUser {
                    login: "user1".into(),
                },
                GithubUser {
                    login: "user2".into(),
                },
            ],
            url: String::new(),
        };
        assert_eq!(issue.first_assignee(), Some("user1"));

        let no_assignee = GithubIssue {
            number: 2,
            title: "Test".into(),
            body: String::new(),
            state: "open".into(),
            labels: vec![],
            assignees: vec![],
            url: String::new(),
        };
        assert_eq!(no_assignee.first_assignee(), None);
    }
}
