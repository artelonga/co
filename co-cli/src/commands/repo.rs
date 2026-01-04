//! Repository management commands - `co repo`
//!
//! Manage registered repositories for federated queries.

use co::{GlobalConfig, RepoLocalConfig};
use colored::Colorize;
use std::path::PathBuf;

/// Repo subcommand action
pub enum RepoAction {
    /// List registered repositories
    List,
    /// Add current directory as a repository
    Add {
        path: PathBuf,
        alias: String,
        ssh_host: Option<String>,
    },
    /// Remove a repository
    Remove { identifier: String },
    /// Add tags to current repository
    Tag { tags: Vec<String> },
    /// Remove tags from current repository
    Untag { tags: Vec<String> },
    /// Switch active workspace context
    Switch { alias: String },
}

/// Run repo command
pub fn run(action: RepoAction) {
    match action {
        RepoAction::List => list_repos(),
        RepoAction::Add {
            path,
            alias,
            ssh_host,
        } => add_repo(path, alias, ssh_host),
        RepoAction::Remove { identifier } => remove_repo(&identifier),
        RepoAction::Tag { tags } => add_tags(tags),
        RepoAction::Untag { tags } => remove_tags(tags),
        RepoAction::Switch { alias } => switch_repo(&alias),
    }
}

/// List registered repositories
fn list_repos() {
    let config = GlobalConfig::load();

    if config.repos.is_empty() {
        println!("{}", "No repositories registered.".yellow());
        println!(
            "{}",
            "Use 'co repo add . --alias <name>' to register the current directory.".dimmed()
        );
        return;
    }

    println!("{}", "Registered Repositories".bold());
    println!("{}", "═".repeat(60));

    // Detect current space for highlighting
    let current_space = config.detect_current_space();

    for repo in &config.repos {
        let local = RepoLocalConfig::load(&repo.path);
        let tags_str = if local.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", local.tags.join(", "))
        };

        let ssh_str = repo
            .ssh_host
            .as_ref()
            .map(|h| format!(" ({})", h.green()))
            .unwrap_or_default();

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
            "{}{} {} {}{}{}{}",
            marker,
            repo.alias.cyan().bold(),
            "→".dimmed(),
            repo.path.display(),
            ssh_str,
            tags_str.dimmed(),
            status
        );
    }
}

/// Add a repository
fn add_repo(path: PathBuf, alias: String, ssh_host: Option<String>) {
    let abs_path = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "{} Cannot resolve path '{}': {}",
                "error:".red().bold(),
                path.display(),
                e
            );
            std::process::exit(1);
        }
    };

    // Check if it looks like a CO repository
    let has_co_dir = abs_path.join(".co").is_dir();
    let has_scopes = std::fs::read_dir(&abs_path)
        .map(|entries| {
            entries.flatten().any(|e| {
                e.path().is_dir()
                    && !e.file_name().to_string_lossy().starts_with('.')
                    && e.file_name() != "target"
            })
        })
        .unwrap_or(false);

    if !has_co_dir && !has_scopes {
        println!(
            "{} Directory doesn't appear to be a CO repository (no .co/ or scope directories)",
            "warning:".yellow().bold()
        );
    }

    let mut config = GlobalConfig::load();
    config.add_repo(abs_path.clone(), alias.clone(), ssh_host.clone());

    if let Err(e) = config.save() {
        eprintln!("{} Failed to save config: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    let ssh_msg = ssh_host
        .map(|h| format!(" with SSH host '{}'", h.green()))
        .unwrap_or_default();

    println!(
        "{} Registered '{}' as '{}'{}",
        "success:".green().bold(),
        abs_path.display(),
        alias.cyan(),
        ssh_msg
    );
}

/// Remove a repository
fn remove_repo(identifier: &str) {
    let mut config = GlobalConfig::load();

    if !config.remove_repo(identifier) {
        eprintln!(
            "{} Repository '{}' not found",
            "error:".red().bold(),
            identifier
        );
        std::process::exit(1);
    }

    if let Err(e) = config.save() {
        eprintln!("{} Failed to save config: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} Removed repository '{}'",
        "success:".green().bold(),
        identifier
    );
}

/// Add tags to current repository
fn add_tags(tags: Vec<String>) {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let mut config = RepoLocalConfig::load(&current_dir);

    for tag in &tags {
        if !config.tags.contains(tag) {
            config.tags.push(tag.clone());
        }
    }

    if let Err(e) = config.save(&current_dir) {
        eprintln!("{} Failed to save config: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} Added tags: {}",
        "success:".green().bold(),
        tags.join(", ").cyan()
    );
}

/// Remove tags from current repository
fn remove_tags(tags: Vec<String>) {
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let mut config = RepoLocalConfig::load(&current_dir);
    let before = config.tags.len();

    config.tags.retain(|t| !tags.contains(t));

    if config.tags.len() == before {
        println!("{}", "No matching tags found.".yellow());
        return;
    }

    if let Err(e) = config.save(&current_dir) {
        eprintln!("{} Failed to save config: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} Removed tags: {}",
        "success:".green().bold(),
        tags.join(", ").cyan()
    );
}

/// Switch active workspace context
fn switch_repo(alias: &str) {
    let mut config = GlobalConfig::load();

    // Handle clearing active repo
    if alias == "none" || alias == "-" {
        config.clear_active_repo();
        if let Err(e) = config.save() {
            eprintln!("{} Failed to save config: {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
        println!("{} Cleared active workspace", "success:".green().bold());
        return;
    }

    // Try to set the active repo
    if !config.set_active_repo(alias) {
        eprintln!(
            "{} Repository '{}' not found. Use 'co repo list' to see registered repos.",
            "error:".red().bold(),
            alias
        );
        std::process::exit(1);
    }

    if let Err(e) = config.save() {
        eprintln!("{} Failed to save config: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    let repo = config.get_by_alias(alias).unwrap();
    println!(
        "{} Switched to '{}' ({})",
        "success:".green().bold(),
        alias.cyan(),
        repo.path.display()
    );
}
