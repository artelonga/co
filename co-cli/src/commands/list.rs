//! List contexts and languages
//!
//! Discovers directories at current location and identifies their types.
//!
//! # Terminology
//!
//! "Context" is the preferred term for agentic interoperability.
//! Supports both "context" and legacy "scope" type in frontmatter.

use colored::Colorize;
use std::fs;
use std::path::Path;

/// Context type detected from README frontmatter
#[derive(Debug)]
pub enum ContextType {
    Language,
    Context,
    Unknown,
}

impl std::fmt::Display for ContextType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextType::Language => write!(f, "language"),
            ContextType::Context => write!(f, "context"),
            ContextType::Unknown => write!(f, "unknown"),
        }
    }
}

/// Detect the type of a directory from its README.md frontmatter
fn detect_type(dir_path: &Path) -> ContextType {
    let readme_path = dir_path.join("README.md");
    if !readme_path.exists() {
        return ContextType::Unknown;
    }

    if let Ok(content) = fs::read_to_string(&readme_path) {
        // Simple frontmatter parsing - look for type: field
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("type:") {
                let type_value = trimmed.strip_prefix("type:").unwrap().trim();
                return match type_value {
                    "language" => ContextType::Language,
                    // Support both "context" and legacy "scope"
                    "context" | "scope" => ContextType::Context,
                    _ => ContextType::Unknown,
                };
            }
        }
    }

    ContextType::Unknown
}

/// Run the list command
pub fn run(stats: bool) {
    println!("{}", "CO Contexts".bold());
    println!("{}", "─".repeat(40));

    let current_dir = Path::new(".");
    let mut found_any = false;

    if let Ok(entries) = fs::read_dir(current_dir) {
        let mut items: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter(|e| {
                // Skip hidden directories and common non-context directories
                let name = e.file_name().to_string_lossy().to_string();
                !name.starts_with('.') && name != "target" && name != "node_modules"
            })
            .collect();

        items.sort_by_key(|a| a.file_name());

        for entry in items {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let context_type = detect_type(&path);

            // Only show directories that have a recognized type
            if let ContextType::Unknown = context_type {
                continue;
            }

            found_any = true;

            if stats {
                // Count files in the directory
                let file_count = count_files(&path);
                println!(
                    "  {} ({}) - {} files",
                    name.cyan(),
                    context_type.to_string().yellow(),
                    file_count
                );
            } else {
                println!("  {} ({})", name.cyan(), context_type.to_string().yellow());
            }
        }
    }

    if !found_any {
        println!("  {}", "No contexts or languages found".dimmed());
    }
}

/// Count files recursively in a directory
fn count_files(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                count += 1;
            } else if path.is_dir() {
                count += count_files(&path);
            }
        }
    }
    count
}
