//! Find command - query content by frontmatter fields
//!
//! Searches all contexts by default, displaying results with context prefixes.
//!
//! Examples:
//!   co locate find status:todo
//!   co locate find type:task

use super::common::is_hidden;
use colored::Colorize;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Content item with context information
struct ContentItem {
    context: String,
    id: String,
}

/// Frontmatter for filtering (captures all string fields)
#[derive(Debug, Deserialize)]
struct FilterableFrontmatter {
    #[serde(flatten)]
    fields: HashMap<String, serde_yaml::Value>,
}

/// Run the find command
pub fn run(filters: &[String], scope_filter: Option<&str>) {
    // Check if first argument is a positional context (no colon = potential context)
    let (positional_context, filter_args): (Option<&str>, &[String]) =
        if !filters.is_empty() && !filters[0].contains(':') {
            // First arg has no colon - treat as context name
            if !is_context_dir(&filters[0]) {
                eprintln!(
                    "{} Context '{}' not found",
                    "error:".red().bold(),
                    filters[0]
                );
                std::process::exit(1);
            }
            (Some(&filters[0]), &filters[1..])
        } else {
            (None, filters)
        };

    // Parse filters from "field:value" format
    let parsed_filters: Vec<(String, String)> = filter_args
        .iter()
        .filter_map(|f| {
            let parts: Vec<&str> = f.splitn(2, ':').collect();
            if parts.len() == 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect();

    // Combine --in flag with positional context (--in takes precedence if both specified)
    let effective_context = scope_filter.or(positional_context);

    // Parse context filter (comma-separated)
    let contexts: Option<Vec<&str>> = effective_context.map(|s| s.split(',').collect());

    // Collect all matching content
    let results = find_all_content(&parsed_filters, contexts.as_deref());

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

/// Find all content matching the filters across contexts
fn find_all_content(filters: &[(String, String)], contexts: Option<&[&str]>) -> Vec<ContentItem> {
    let mut results = Vec::new();
    let current_dir = Path::new(".");

    // Iterate through all directories (potential contexts)
    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                let context_name = path.file_name().unwrap().to_string_lossy().to_string();

                // Skip if context filter is set and this context is not in the list
                if let Some(allowed_contexts) = contexts {
                    if !allowed_contexts.contains(&context_name.as_str()) {
                        continue;
                    }
                }

                // Search in type subdirectories (tasks/, definitions/, etc.)
                if let Ok(type_dirs) = fs::read_dir(&path) {
                    for type_dir in type_dirs.flatten() {
                        let type_path = type_dir.path();
                        if type_path.is_dir() {
                            search_directory(&type_path, &context_name, filters, &mut results);
                        }
                    }
                }

                // Also search directly in the context root
                search_directory(&path, &context_name, filters, &mut results);
            }
        }
    }

    results
}

/// Search a directory for matching content files
fn search_directory(
    dir: &Path,
    context: &str,
    filters: &[(String, String)],
    results: &mut Vec<ContentItem>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == "md") {
                if let Some(item) = check_file(&path, context, filters) {
                    results.push(item);
                }
            }
        }
    }
}

/// Check if a file matches the filters
fn check_file(path: &Path, context: &str, filters: &[(String, String)]) -> Option<ContentItem> {
    let content = fs::read_to_string(path).ok()?;

    // Extract frontmatter
    if !content.starts_with("---") {
        return None;
    }

    let end_idx = content[3..].find("\n---")?;
    let yaml_str = &content[4..end_idx + 3];

    let frontmatter: FilterableFrontmatter = serde_yaml::from_str(yaml_str).ok()?;

    // Convert fields to string map for comparison
    let mut string_fields: HashMap<String, String> = HashMap::new();
    for (key, value) in &frontmatter.fields {
        if let Some(s) = value.as_str() {
            string_fields.insert(key.clone(), s.to_string());
        }
    }

    // Check all filters match
    for (field, value) in filters {
        match string_fields.get(field) {
            Some(v) if v == value => continue,
            _ => return None,
        }
    }

    // Get ID from frontmatter or filename
    let id = string_fields
        .get("id")
        .cloned()
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });

    Some(ContentItem {
        context: context.to_string(),
        id,
    })
}

/// Check if a name corresponds to an existing context directory
fn is_context_dir(name: &str) -> bool {
    let path = Path::new(name);
    path.is_dir() && !is_hidden(path)
}
