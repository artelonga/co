//! Update content files
//!
//! Updates frontmatter fields in content files.

use crate::i18n::load_i18n;
use colored::Colorize;
use std::fs;
use std::path::Path;

/// Run the update command
pub fn run(name: &str, status: Option<&str>) {
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

            let updated_content = if let Some(new_status) = status {
                update_frontmatter_field(&content, "status", new_status)
            } else {
                content
            };

            if let Err(e) = fs::write(&path, updated_content) {
                eprintln!(
                    "{} Failed to write {}: {}",
                    "error:".red().bold(),
                    path.display(),
                    e
                );
                std::process::exit(1);
            }

            println!("{}", "Updated".bold().green());
            println!("{}", "─".repeat(30));
            println!("  {}: {}", "file".cyan(), path.display());
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

/// Update a field in frontmatter
fn update_frontmatter_field(content: &str, field: &str, value: &str) -> String {
    let pattern = format!("{}: ", field);
    let mut result = String::new();
    let mut in_frontmatter = false;
    let mut field_updated = false;

    for line in content.lines() {
        if line == "---" {
            in_frontmatter = !in_frontmatter;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if in_frontmatter && line.starts_with(&pattern) {
            result.push_str(&format!("{}: {}", field, value));
            result.push('\n');
            field_updated = true;
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // Remove trailing newline if original didn't have one
    if !content.ends_with('\n') {
        result.pop();
    }

    if !field_updated {
        // Field didn't exist, we could add it (for now just return as-is)
    }

    result
}
