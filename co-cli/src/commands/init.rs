//! Initialize a new context (project space)
//!
//! Creates a context directory at the repository root.
//! All .md files within become lexicon entries, traversed recursively.
//!
//! Spaces are automatically added to .gitignore to prevent accidental
//! commits to the co home repository.

use crate::i18n::load_i18n;
use colored::Colorize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Run the init command
pub fn run(name: &str) {
    let i18n = load_i18n();
    let context_path = Path::new(name);

    // Check if context already exists
    if context_path.exists() {
        eprintln!(
            "{} {} '{}' {}",
            "error:".red().bold(),
            i18n.type_label("context"),
            name,
            i18n.message("already_exists")
        );
        std::process::exit(1);
    }

    // Create context directory
    if let Err(e) = fs::create_dir_all(context_path) {
        eprintln!(
            "{} Failed to create {}: {}",
            "error:".red().bold(),
            context_path.display(),
            e
        );
        std::process::exit(1);
    }

    // Create README.md with frontmatter
    let readme_content = format!(
        r#"---
type: context
id: {}
language: english
---

# {}

A CO context for content management.
"#,
        name, name
    );

    let readme_path = context_path.join("README.md");
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

    println!("{}", i18n.message("context_initialized").bold().green());
    println!("{}", "─".repeat(30));
    println!("Created: {}/", name.cyan());
    println!("  └── README.md");

    if added_to_gitignore {
        println!("  └── {} added to .gitignore", gitignore_entry.dimmed());
    }
}
