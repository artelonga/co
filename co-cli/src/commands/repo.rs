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
    },
    /// Remove a repository
    Remove {
        identifier: String,
    },
    /// Add tags to current repository
    Tag {
        tags: Vec<String>,
    },
    /// Remove tags from current repository
    Untag {
        tags: Vec<String>,
    },
}

/// Run repo command
pub fn run(action: RepoAction) {
    match action {
        RepoAction::List => list_repos(),
        RepoAction::Add { path, alias } => add_repo(path, alias),
        RepoAction::Remove { identifier } => remove_repo(&identifier),
        RepoAction::Tag { tags } => add_tags(tags),
        RepoAction::Untag { tags } => remove_tags(tags),
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

    for repo in &config.repos {
        let local = RepoLocalConfig::load(&repo.path);
        let tags_str = if local.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", local.tags.join(", "))
        };

        let exists = repo.path.exists();
        let status = if exists {
            "".to_string()
        } else {
            " (not found)".red().to_string()
        };

        println!(
            "  {} {} {}{}{}",
            repo.alias.cyan().bold(),
            "→".dimmed(),
            repo.path.display(),
            tags_str.dimmed(),
            status
        );
    }
}

/// Add a repository
fn add_repo(path: PathBuf, alias: String) {
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
    config.add_repo(abs_path.clone(), alias.clone());

    if let Err(e) = config.save() {
        eprintln!("{} Failed to save config: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} Registered '{}' as '{}'",
        "success:".green().bold(),
        abs_path.display(),
        alias.cyan()
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
