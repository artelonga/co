//! GitHub CLI integration for issue synchronization
//!
//! This module provides a wrapper around the `gh` CLI tool for
//! fetching and managing GitHub issues.
//!
//! # Architecture
//!
//! - `GhCli` - Low-level wrapper around the `gh` command
//! - `types` - GitHub data structures (Issue, Label, etc.)
//! - `mapping` - Conversion between GitHub issues and CO content

pub mod mapping;
pub mod types;

pub use mapping::MappedWorkItem;
pub use types::{GithubIssue, GithubLabel, GithubUser};

use std::process::Command;

/// Error type for GitHub CLI operations
#[derive(Debug)]
pub enum GhError {
    /// The `gh` CLI is not installed
    NotInstalled,
    /// The user is not authenticated
    NotAuthenticated,
    /// Command execution failed
    CommandFailed(String),
    /// Failed to parse JSON output
    ParseError(String),
}

impl std::fmt::Display for GhError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GhError::NotInstalled => write!(
                f,
                "GitHub CLI (gh) is not installed. Install from https://cli.github.com"
            ),
            GhError::NotAuthenticated => {
                write!(
                    f,
                    "Not authenticated with GitHub. Run 'gh auth login' first."
                )
            }
            GhError::CommandFailed(msg) => write!(f, "GitHub CLI command failed: {}", msg),
            GhError::ParseError(msg) => write!(f, "Failed to parse GitHub response: {}", msg),
        }
    }
}

impl std::error::Error for GhError {}

/// Wrapper around the `gh` CLI tool
pub struct GhCli;

impl GhCli {
    /// Check if `gh` CLI is available and authenticated
    pub fn check_available() -> Result<(), GhError> {
        // Check if gh is installed
        let version_check = Command::new("gh").arg("--version").output();

        match version_check {
            Ok(output) if output.status.success() => {}
            _ => return Err(GhError::NotInstalled),
        }

        // Check if authenticated
        let auth_check = Command::new("gh")
            .args(["auth", "status"])
            .output()
            .map_err(|_| GhError::NotInstalled)?;

        if !auth_check.status.success() {
            return Err(GhError::NotAuthenticated);
        }

        Ok(())
    }

    /// List issues from the current repository
    ///
    /// # Arguments
    /// * `state` - Filter by state: "open", "closed", or "all"
    /// * `labels` - Filter by labels (optional)
    /// * `limit` - Maximum number of issues to return
    pub fn issue_list(
        state: &str,
        labels: Option<&[&str]>,
        limit: u32,
    ) -> Result<Vec<GithubIssue>, GhError> {
        let mut args = vec![
            "issue",
            "list",
            "--json",
            "number,title,body,state,labels,assignees,url",
            "--state",
            state,
            "--limit",
        ];

        let limit_str = limit.to_string();
        args.push(&limit_str);

        // Add label filters
        let label_args: Vec<String>;
        if let Some(labels) = labels {
            label_args = labels.iter().map(|l| format!("--label={}", l)).collect();
            for arg in &label_args {
                args.push(arg);
            }
        }

        let output = Command::new("gh")
            .args(&args)
            .output()
            .map_err(|e| GhError::CommandFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GhError::CommandFailed(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout).map_err(|e| GhError::ParseError(e.to_string()))
    }

    /// Get a single issue by number
    pub fn issue_view(number: u64) -> Result<GithubIssue, GhError> {
        let output = Command::new("gh")
            .args([
                "issue",
                "view",
                &number.to_string(),
                "--json",
                "number,title,body,state,labels,assignees,url",
            ])
            .output()
            .map_err(|e| GhError::CommandFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GhError::CommandFailed(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout).map_err(|e| GhError::ParseError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gh_error_display() {
        let err = GhError::NotInstalled;
        assert!(err.to_string().contains("not installed"));

        let err = GhError::NotAuthenticated;
        assert!(err.to_string().contains("gh auth login"));

        let err = GhError::CommandFailed("failed".into());
        assert!(err.to_string().contains("failed"));

        let err = GhError::ParseError("invalid json".into());
        assert!(err.to_string().contains("invalid json"));
    }
}
