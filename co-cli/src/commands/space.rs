//! Space management commands - `co space`
//!
//! Manage spaces (contexts) for multi-repo workflows.
//! A space can be a registered git repo, a private non-git folder,
//! or any directory with `.co/` configuration.

use co::{GlobalConfig, RepoLocalConfig};
use colored::Colorize;

/// Space subcommand action
pub enum SpaceAction {
    /// List all registered spaces
    List,
    /// Show current space details
    Current,
}

/// Run space command
pub fn run(action: SpaceAction) {
    match action {
        SpaceAction::List => list_spaces(),
        SpaceAction::Current => show_current_space(),
    }
}

/// List all registered spaces
fn list_spaces() {
    let config = GlobalConfig::load();

    if config.repos.is_empty() {
        println!("{}", "No spaces registered.".yellow());
        println!(
            "{}",
            "Use 'co repo add . --alias <name>' to register a space.".dimmed()
        );
        return;
    }

    println!("{}", "Spaces".bold());
    println!("{}", "═".repeat(60));

    // Detect current space for highlighting
    let current_space = config.detect_current_space();

    for repo in &config.repos {
        let local = RepoLocalConfig::load(&repo.path);

        // Determine space type
        let is_git = repo.path.join(".git").exists();
        let space_type = if is_git { "[repo]" } else { "[private]" };

        // Build tags string
        let tags_str = if local.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", local.tags.join(", "))
        };

        // Build SSH host string
        let ssh_str = repo
            .ssh_host
            .as_ref()
            .map(|h| format!(" {}", h.green()))
            .unwrap_or_default();

        // Check if path exists
        let exists = repo.path.exists();
        let status = if exists {
            "".to_string()
        } else {
            " (not found)".red().to_string()
        };

        // Mark current space with *
        let marker = if current_space.is_some_and(|c| c.path == repo.path) {
            "* ".green().to_string()
        } else {
            "  ".to_string()
        };

        println!(
            "{}{:<12} {} {} {}{}{}",
            marker,
            repo.alias.cyan().bold(),
            repo.path.display(),
            space_type.dimmed(),
            ssh_str,
            tags_str.dimmed(),
            status
        );
    }
}

/// Show current space details
fn show_current_space() {
    let config = GlobalConfig::load();

    // Check if we're in a detected space (from cwd)
    let detected = config.detect_current_space();
    let is_switched = detected.is_none() && config.active_repo.is_some();

    // Get effective space (detected or active)
    let Some(space) = config.effective_space() else {
        // Not in any space - show helpful message
        show_not_in_workspace_help(&config);
        return;
    };

    let local = RepoLocalConfig::load(&space.path);

    // Determine space type
    let is_git = space.path.join(".git").exists();
    let space_type = if is_git { "repo" } else { "private" };

    // Show header with switched indicator if applicable
    if is_switched {
        println!(
            "{} {} {}",
            "Current Space:".bold(),
            space.alias.cyan().bold(),
            "(switched)".dimmed()
        );
    } else {
        println!("{} {}", "Current Space:".bold(), space.alias.cyan().bold());
    }
    println!("{}", "─".repeat(40));
    println!("  {:<10} {}", "Path:".dimmed(), space.path.display());
    println!("  {:<10} {}", "Type:".dimmed(), space_type);

    if let Some(ssh_host) = &space.ssh_host {
        println!("  {:<10} {}", "SSH:".dimmed(), ssh_host.green());
    }

    if !local.tags.is_empty() {
        println!("  {:<10} {}", "Tags:".dimmed(), local.tags.join(", "));
    }

    // List contexts (subdirectories that could be scopes)
    if let Ok(entries) = std::fs::read_dir(&space.path) {
        let contexts: Vec<String> = entries
            .flatten()
            .filter(|e| {
                e.path().is_dir()
                    && !e.file_name().to_string_lossy().starts_with('.')
                    && e.file_name() != "target"
                    && e.file_name() != "node_modules"
            })
            .map(|e| format!("{}/", e.file_name().to_string_lossy()))
            .collect();

        if !contexts.is_empty() {
            println!("  {:<10} {}", "Contexts:".dimmed(), contexts.join(", "));
        }
    }
}

/// Show helpful message when not in a CO workspace
fn show_not_in_workspace_help(config: &GlobalConfig) {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".to_string());

    println!("{}", "Not in a CO workspace.".yellow().bold());
    println!();
    println!("{}", "To use CO, you can:".dimmed());
    println!("  1. {} to a registered workspace", "Navigate".cyan());
    println!(
        "  2. {} this directory: {}",
        "Register".cyan(),
        format!("co repo add {} --alias <name>", cwd).dimmed()
    );
    println!(
        "  3. {} to a registered space: {}",
        "Switch".cyan(),
        "co repo switch <alias>".dimmed()
    );
    println!();

    if config.repos.is_empty() {
        println!("{}", "No workspaces registered yet.".dimmed());
    } else {
        println!("{}", "Registered workspaces:".dimmed());
        for repo in &config.repos {
            println!("  {} → {}", repo.alias.cyan(), repo.path.display());
        }
    }
}
