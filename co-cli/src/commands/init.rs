//! Initialize a new namespace directory
//!
//! Creates a simple namespace directory where users can organize
//! their content files in whatever format they prefer.

use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Run the init command
pub fn run(name: &str) {
    let space_path = Path::new(name);

    // Check if directory already exists
    if space_path.exists() {
        eprintln!(
            "{} Directory '{}' already exists",
            "error:".red().bold(),
            name
        );
        std::process::exit(1);
    }

    // Just create the directory - user organizes files however they want
    if let Err(e) = fs::create_dir_all(space_path) {
        eprintln!(
            "{} Failed to create directory: {}",
            "error:".red().bold(),
            e
        );
        std::process::exit(1);
    }

    println!(
        "{} Created namespace '{}'",
        "success:".green().bold(),
        name.cyan()
    );
}

/// Check for spaces that exist but aren't gitignored (commit guard)
pub fn check_unprotected_spaces() {
    let gitignore_path = Path::new(".gitignore");

    // Load gitignored patterns
    let gitignored: HashSet<String> = if gitignore_path.exists() {
        fs::read_to_string(gitignore_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| l.trim().trim_end_matches('/').to_string())
            .collect()
    } else {
        HashSet::new()
    };

    // Find directories that look like spaces (have README with type: space)
    let mut unprotected = Vec::new();

    if let Ok(entries) = fs::read_dir(".") {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden directories and common non-space dirs
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }

            // Check if it looks like a space (has README with type: space)
            let readme_path = path.join("README.md");
            if readme_path.exists()
                && let Ok(content) = fs::read_to_string(&readme_path)
                && content.contains("type: space")
                && !gitignored.contains(&name)
            {
                unprotected.push(name);
            }
        }
    }

    if unprotected.is_empty() {
        println!("{}", "All spaces are protected.".green());
    } else {
        println!("{}", "Unprotected spaces (not gitignored):".yellow().bold());
        for name in &unprotected {
            println!("  {} {}", "⚠".yellow(), name);
        }
        println!(
            "\n{}",
            "Add these to .gitignore to prevent accidental commits.".dimmed()
        );
    }
}
