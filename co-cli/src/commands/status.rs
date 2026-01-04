//! Status command - show graph statistics

use co::{LANGUAGES, SpaceLocation};
use colored::Colorize;
use std::path::Path;

pub fn run() {
    println!("{}", "CO Graph Status".bold().green());
    println!("{}", "═".repeat(40));

    // Show current space location (commit guard context)
    let cwd = std::env::current_dir().unwrap_or_default();
    let location = SpaceLocation::detect(&cwd);
    println!("\n{}", "Location:".bold());
    match &location {
        SpaceLocation::InSpace {
            space_name,
            space_path,
            repo_root,
        } => {
            println!("  {} {} (space)", "◆".cyan(), space_name.cyan().bold());
            println!("    Path: {}", space_path.display());
            if let Some(root) = repo_root {
                println!("    Repo: {}", root.display().to_string().dimmed());
            }
        }
        SpaceLocation::AtRepoRoot { repo_root } => {
            println!("  {} {} (repo root)", "◇".yellow(), format_path(repo_root));
            println!(
                "    {}",
                "Not inside a space - commits will affect the repository.".dimmed()
            );
        }
        SpaceLocation::Unknown => {
            println!("  {} {}", "?".dimmed(), "Unknown location".dimmed());
        }
    }

    println!("\n{}", "Languages:".bold());
    for lang in LANGUAGES {
        println!("  • {}", lang);
    }

    println!("\n{}", "Index:".bold());
    println!("  Entries: {}", "0 (not built)".yellow());

    println!("\n{}", "Storage:".bold());
    println!("  Root: {}", "./graph/".cyan());
}

/// Format a path for display (show last component or full path if at root)
fn format_path(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}
