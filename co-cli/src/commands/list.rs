//! List spaces and languages
//!
//! Discovers directories at current location and identifies their types.
//!
//! # Terminology
//!
//! "Space" is the canonical term for namespace directories.
//! Supports "space" and legacy "context"/"scope" types in frontmatter.

use colored::Colorize;
use std::fs;
use std::path::Path;

/// Space type detected from README frontmatter
#[derive(Debug)]
pub enum SpaceType {
    Language,
    Space,
    Unknown,
}

impl std::fmt::Display for SpaceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpaceType::Language => write!(f, "language"),
            SpaceType::Space => write!(f, "space"),
            SpaceType::Unknown => write!(f, "unknown"),
        }
    }
}

/// Detect the type of a directory from its README.md frontmatter
fn detect_type(dir_path: &Path) -> SpaceType {
    let readme_path = dir_path.join("README.md");
    if !readme_path.exists() {
        return SpaceType::Unknown;
    }

    if let Ok(content) = fs::read_to_string(&readme_path) {
        // Simple frontmatter parsing - look for type: field
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("type:") {
                let type_value = trimmed.strip_prefix("type:").unwrap().trim();
                return match type_value {
                    "language" => SpaceType::Language,
                    // Support "space" and legacy "context"/"scope"
                    "space" | "context" | "scope" => SpaceType::Space,
                    _ => SpaceType::Unknown,
                };
            }
        }
    }

    SpaceType::Unknown
}

/// Run the list command
pub fn run(stats: bool) {
    println!("{}", "CO Spaces".bold());
    println!("{}", "─".repeat(40));

    let current_dir = Path::new(".");
    let mut found_any = false;

    if let Ok(entries) = fs::read_dir(current_dir) {
        let mut items: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter(|e| {
                // Skip hidden directories and common non-space directories
                let name = e.file_name().to_string_lossy().to_string();
                !name.starts_with('.') && name != "target" && name != "node_modules"
            })
            .collect();

        items.sort_by_key(|a| a.file_name());

        for entry in items {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let space_type = detect_type(&path);

            // Only show directories that have a recognized type
            if let SpaceType::Unknown = space_type {
                continue;
            }

            found_any = true;

            if stats {
                // Count files in the directory
                let file_count = count_files(&path);
                println!(
                    "  {} ({}) - {} files",
                    name.cyan(),
                    space_type.to_string().yellow(),
                    file_count
                );
            } else {
                println!("  {} ({})", name.cyan(), space_type.to_string().yellow());
            }
        }
    }

    if !found_any {
        println!("  {}", "No spaces or languages found".dimmed());
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
