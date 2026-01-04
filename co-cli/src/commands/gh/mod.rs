//! GitHub CLI wrapper commands - `co gh`
//!
//! Low-level wrapper around the `gh` CLI tool for GitHub operations.

pub mod issue;

use colored::Colorize;

/// GitHub subcommand action
pub enum GhAction {
    /// Issue operations
    Issue { action: issue::IssueAction },
}

/// Run gh command
pub fn run(action: GhAction) {
    // Check if gh is available
    if let Err(e) = co::GhCli::check_available() {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    match action {
        GhAction::Issue { action } => issue::run(action),
    }
}
