//! Search command - full-text search in content body
//!
//! Searches the body text of all content files.
//!
//! Examples:
//!   co locate search "important meeting"
//!   co locate search api

use super::common::is_hidden;
use colored::Colorize;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Content item with context information
struct SearchResult {
    context: String,
    id: String,
}

/// Frontmatter for extracting ID
#[derive(Debug, Deserialize)]
struct BasicFrontmatter {
    #[serde(flatten)]
    fields: HashMap<String, serde_yaml::Value>,
}

/// Run the search command
pub fn run(query: &str) {
    let results = search_all_content(query);

    if results.is_empty() {
        println!("{}", "No results found".yellow());
        return;
    }

    // Display results with context prefix
    for item in results {
        println!(
            "{} {}",
            format!("[{}]", item.context).cyan(),
            item.id.white()
        );
    }
}

/// Search all content for the query string in the body
fn search_all_content(query: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let current_dir = Path::new(".");
    let query_lower = query.to_lowercase();

    // Iterate through all directories (potential contexts)
    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                let context_name = path.file_name().unwrap().to_string_lossy().to_string();

                // Search in type subdirectories (tasks/, definitions/, etc.)
                if let Ok(type_dirs) = fs::read_dir(&path) {
                    for type_dir in type_dirs.flatten() {
                        let type_path = type_dir.path();
                        if type_path.is_dir() {
                            search_directory(&type_path, &context_name, &query_lower, &mut results);
                        }
                    }
                }

                // Also search directly in the context root
                search_directory(&path, &context_name, &query_lower, &mut results);
            }
        }
    }

    results
}

/// Search a directory for files containing the query in body
fn search_directory(
    dir: &Path,
    context: &str,
    query: &str,
    results: &mut Vec<SearchResult>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == "md") {
                if let Some(item) = check_file(&path, context, query) {
                    results.push(item);
                }
            }
        }
    }
}

/// Check if a file's body contains the query
fn check_file(path: &Path, context: &str, query: &str) -> Option<SearchResult> {
    let content = fs::read_to_string(path).ok()?;

    // Extract frontmatter and body
    if !content.starts_with("---") {
        // No frontmatter, search entire content
        if content.to_lowercase().contains(query) {
            let id = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            return Some(SearchResult {
                context: context.to_string(),
                id,
            });
        }
        return None;
    }

    let end_idx = content[3..].find("\n---")?;
    let yaml_str = &content[4..end_idx + 3];
    let body_start = end_idx + 7; // Skip past "\n---\n"
    let body = if body_start < content.len() {
        &content[body_start..]
    } else {
        ""
    };

    // Search in body only (not frontmatter)
    if !body.to_lowercase().contains(query) {
        return None;
    }

    // Extract ID from frontmatter
    let frontmatter: BasicFrontmatter = serde_yaml::from_str(yaml_str).ok()?;
    let id = frontmatter
        .fields
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });

    Some(SearchResult {
        context: context.to_string(),
        id,
    })
}

