//! Collaboration commands - `co collab`
//!
//! High-level commands for syncing GitHub issues with local content.

pub mod pull;

use colored::Colorize;

/// Collab subcommand action
pub enum CollabAction {
    /// Pull GitHub issues to local files
    Pull { all: bool, numbers: Vec<u64> },
}

/// Run collab command
pub fn run(action: CollabAction) {
    // Check if gh is available
    if let Err(e) = co::GhCli::check_available() {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    match action {
        CollabAction::Pull { all, numbers } => pull::run(all, numbers),
    }
}
