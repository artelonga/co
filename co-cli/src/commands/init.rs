//! Initialize a new context
//!
//! Creates a context directory at the repository root.
//! All .md files within become lexicon entries, traversed recursively.
//!
//! # Terminology
//!
//! "Context" is the preferred term for agentic interoperability.
//! The term "scope" is deprecated but may be aliased for backwards compatibility.

use crate::i18n::load_i18n;
use colored::Colorize;
use std::fs;
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

    println!("{}", i18n.message("context_initialized").bold().green());
    println!("{}", "─".repeat(30));
    println!("Created: {}/", name.cyan());
    println!("  └── README.md");
}
