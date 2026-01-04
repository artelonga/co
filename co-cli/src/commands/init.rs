//! Initialize a new space
//!
//! Creates a space directory at the repository root.
//! All .md files within become lexicon entries, traversed recursively.
//!
//! Spaces are automatically added to .gitignore to prevent accidental
//! commits to the co home repository.

use crate::i18n::load_i18n;
use colored::Colorize;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Run the init command
pub fn run(name: &str) {
    let i18n = load_i18n();
    let space_path = Path::new(name);

    // Check if space already exists
    if space_path.exists() {
        eprintln!(
            "{} {} '{}' {}",
            "error:".red().bold(),
            i18n.type_label("space"),
            name,
            i18n.message("already_exists")
        );
        std::process::exit(1);
    }

    // Create space directory
    if let Err(e) = fs::create_dir_all(space_path) {
        eprintln!(
            "{} Failed to create {}: {}",
            "error:".red().bold(),
            space_path.display(),
            e
        );
        std::process::exit(1);
    }

    // Create README.md with frontmatter
    let readme_content = format!(
        r#"---
type: space
id: {}
language: english
---

# {}

A CO space for content management.
"#,
        name, name
    );

    let readme_path = space_path.join("README.md");
    if let Err(e) = fs::write(&readme_path, readme_content) {
        eprintln!(
            "{} Failed to create README.md: {}",
            "error:".red().bold(),
            e
        );
        std::process::exit(1);
    }

    // Add to .gitignore (if exists) to prevent accidental commits to co home
    let gitignore_path = Path::new(".gitignore");
    let gitignore_entry = format!("{}/", name);
    let mut added_to_gitignore = false;

    if gitignore_path.exists()
        && let Ok(contents) = fs::read_to_string(gitignore_path)
    {
        if !contents.lines().any(|line| line.trim() == gitignore_entry) {
            if let Ok(mut file) = OpenOptions::new().append(true).open(gitignore_path) {
                let entry = format!("\n# Space: {}\n{}\n", name, gitignore_entry);
                if file.write_all(entry.as_bytes()).is_ok() {
                    added_to_gitignore = true;
                }
            }
        } else {
            added_to_gitignore = true;
        }
    }

    println!("{}", i18n.message("space_initialized").bold().green());
    println!("{}", "─".repeat(30));
    println!("Created: {}/", name.cyan());
    println!("  └── README.md");

    if added_to_gitignore {
        println!("  └── {} added to .gitignore", gitignore_entry.dimmed());
    }
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
