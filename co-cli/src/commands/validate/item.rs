//! Validate a specific content file

use co::{Severity, ValidationContext};
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Run validation on a specific item
pub fn run(name: &str) {
    let current_dir = Path::new(".");

    // Find the file
    let file_path = match find_file(current_dir, name) {
        Some(path) => path,
        None => {
            eprintln!("{} Item '{}' not found", "error:".red().bold(), name);
            std::process::exit(1);
        }
    };

    // Build validation context
    let mut ctx = ValidationContext::new(current_dir);

    // Collect all known IDs for link validation
    collect_known_ids(current_dir, &mut ctx.known_ids);

    // Validate the file
    let issues = co::validate::validate_file(&file_path, &ctx);

    if issues.is_empty() {
        println!("{} {} is valid", "✓".green().bold(), file_path.display());
    } else {
        let mut error_count = 0;
        let mut warning_count = 0;

        for issue in &issues {
            match issue.severity {
                Severity::Error => {
                    println!("{} {}", "ERROR:".red().bold(), issue.message);
                    error_count += 1;
                }
                Severity::Warning => {
                    println!("{} {}", "WARNING:".yellow().bold(), issue.message);
                    warning_count += 1;
                }
            }
        }

        println!();
        println!(
            "Validation complete: {} errors, {} warnings",
            error_count, warning_count
        );

        if error_count > 0 {
            std::process::exit(1);
        }
    }
}

/// Find a file by name across all contexts
fn find_file(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    // Try with .md extension if not provided
    let search_name = if name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{}.md", name)
    };

    // Search in all contexts
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && !is_hidden(&path)
                && let Some(found) = find_in_context(&path, &search_name, name)
            {
                return Some(found);
            }
        }
    }

    None
}

/// Find file in a context directory
fn find_in_context(context_path: &Path, search_name: &str, id: &str) -> Option<std::path::PathBuf> {
    // Check type subdirectories
    if let Ok(entries) = fs::read_dir(context_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find_in_directory(&path, search_name, id) {
                    return Some(found);
                }
            } else if path.is_file() && matches_file(&path, search_name, id) {
                return Some(path);
            }
        }
    }

    None
}

/// Find file in a directory
fn find_in_directory(dir: &Path, search_name: &str, id: &str) -> Option<std::path::PathBuf> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && matches_file(&path, search_name, id) {
                return Some(path);
            }
        }
    }

    None
}

/// Check if file matches the search criteria
fn matches_file(path: &Path, search_name: &str, id: &str) -> bool {
    // Check filename match
    if let Some(filename) = path.file_name().and_then(|n| n.to_str())
        && filename == search_name
    {
        return true;
    }

    // Check ID in frontmatter
    if path.extension().is_some_and(|e| e == "md")
        && let Ok(content) = fs::read_to_string(path)
        && let Some(rest) = content.strip_prefix("---")
        && let Some(end_idx) = rest.find("\n---")
    {
        let yaml_str = &rest[1..end_idx];
        for line in yaml_str.lines() {
            let line = line.trim();
            if let Some(id_value) = line.strip_prefix("id:") {
                let file_id = id_value.trim().trim_matches('"').trim_matches('\'');
                if file_id == id {
                    return true;
                }
            }
        }
    }

    false
}

/// Collect all known content IDs
fn collect_known_ids(root: &Path, known_ids: &mut HashSet<String>) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                collect_ids_from_context(&path, known_ids);
            }
        }
    }
}

/// Collect IDs from a context directory
fn collect_ids_from_context(context_path: &Path, known_ids: &mut HashSet<String>) {
    if let Ok(entries) = fs::read_dir(context_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_ids_from_directory(&path, known_ids);
            } else if path.is_file()
                && path.extension().is_some_and(|e| e == "md")
                && let Some(id) = extract_id(&path)
            {
                known_ids.insert(id);
            }
        }
    }
}

/// Collect IDs from a directory
fn collect_ids_from_directory(dir: &Path, known_ids: &mut HashSet<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|e| e == "md")
                && let Some(id) = extract_id(&path)
            {
                known_ids.insert(id);
            }
        }
    }
}

/// Extract ID from a file's frontmatter
fn extract_id(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let rest = content.strip_prefix("---")?;
    let end_idx = rest.find("\n---")?;
    let yaml_str = &rest[1..end_idx];

    for line in yaml_str.lines() {
        let line = line.trim();
        if let Some(id_value) = line.strip_prefix("id:") {
            let id = id_value.trim().trim_matches('"').trim_matches('\'');
            return Some(id.to_string());
        }
    }

    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

/// Check if a path is hidden
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}
