//! Show content files
//!
//! Displays content from files in scopes.

use crate::i18n::load_i18n;
use colored::Colorize;
use std::fs;
use std::path::Path;

/// Run the show command
pub fn run(name: &str, meta_only: bool) {
    let i18n = load_i18n();

    // Search for the file in all scopes
    let file_path = find_content_file(name);

    match file_path {
        Some(path) => {
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "{} Failed to read {}: {}",
                        "error:".red().bold(),
                        path.display(),
                        e
                    );
                    std::process::exit(1);
                }
            };

            if meta_only {
                // Extract and display only frontmatter
                if let Some(frontmatter) = extract_frontmatter(&content) {
                    println!("{}", frontmatter);
                } else {
                    eprintln!("{} No frontmatter found", "error:".red().bold());
                    std::process::exit(1);
                }
            } else {
                // Display full content
                println!("{}", content);
            }
        }
        None => {
            eprintln!(
                "{} {} '{}' not found",
                "error:".red().bold(),
                i18n.type_label("content"),
                name
            );
            std::process::exit(1);
        }
    }
}

/// Find a content file by name across all scopes
fn find_content_file(name: &str) -> Option<std::path::PathBuf> {
    let current_dir = Path::new(".");

    // List all directories that could be scopes
    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Search in type subdirectories (tasks/, definitions/, etc.)
                if let Ok(type_dirs) = fs::read_dir(&path) {
                    for type_dir in type_dirs.flatten() {
                        let type_path = type_dir.path();
                        if type_path.is_dir() {
                            let file_path = type_path.join(format!("{}.md", name));
                            if file_path.exists() {
                                return Some(file_path);
                            }
                        }
                    }
                }
                // Also check directly in the scope
                let direct_path = path.join(format!("{}.md", name));
                if direct_path.exists() {
                    return Some(direct_path);
                }
            }
        }
    }

    None
}

/// Extract frontmatter from content
fn extract_frontmatter(content: &str) -> Option<&str> {
    let rest = content.strip_prefix("---")?;
    let end = rest.find("---")?;
    Some(&rest[1..end])
}
