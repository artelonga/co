//! Validate all content files

use co::{Severity, ValidationContext};
use colored::Colorize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Run validation on all content files
pub fn run() {
    let current_dir = Path::new(".");

    // Build validation context
    let mut ctx = ValidationContext::new(current_dir);

    // First pass: collect all known IDs
    collect_known_ids(current_dir, &mut ctx.known_ids);

    // Second pass: validate all files
    let mut error_count = 0;
    let mut warning_count = 0;

    let issues = validate_all_files(current_dir, &ctx);

    // Group issues by file
    let mut current_file = String::new();
    for issue in &issues {
        let file_str = issue.file.display().to_string();
        if file_str != current_file {
            current_file = file_str.clone();
            println!();
            match issue.severity {
                Severity::Error => {
                    println!("{} {}", "ERROR:".red().bold(), file_str.white());
                }
                Severity::Warning => {
                    println!("{} {}", "WARNING:".yellow().bold(), file_str.white());
                }
            }
        }
        println!("  {}", issue.message);

        match issue.severity {
            Severity::Error => error_count += 1,
            Severity::Warning => warning_count += 1,
        }
    }

    // Count files
    let file_count = count_content_files(current_dir);

    println!();
    if error_count == 0 && warning_count == 0 {
        println!(
            "{} {} files validated, no issues found",
            "✓".green().bold(),
            file_count
        );
    } else {
        println!(
            "Validation complete: {} errors, {} warnings",
            error_count, warning_count
        );
    }
}

/// Collect all known content IDs from the repository
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
    // Search in type subdirectories (tasks/, definitions/, etc.)
    if let Ok(entries) = fs::read_dir(context_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_ids_from_directory(&path, known_ids);
            } else if path.is_file() && path.extension().map_or(false, |e| e == "md") {
                if let Some(id) = extract_id(&path) {
                    known_ids.insert(id);
                }
            }
        }
    }
}

/// Collect IDs from a directory
fn collect_ids_from_directory(dir: &Path, known_ids: &mut HashSet<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == "md") {
                if let Some(id) = extract_id(&path) {
                    known_ids.insert(id);
                }
            }
        }
    }
}

/// Extract ID from a file's frontmatter
fn extract_id(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    if !content.starts_with("---") {
        return None;
    }

    let end_idx = content[3..].find("\n---")?;
    let yaml_str = &content[4..end_idx + 3];

    // Simple extraction - look for "id:" line
    for line in yaml_str.lines() {
        let line = line.trim();
        if line.starts_with("id:") {
            let id = line[3..].trim().trim_matches('"').trim_matches('\'');
            return Some(id.to_string());
        }
    }

    // Fall back to filename
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

/// Validate all files and return issues
fn validate_all_files(root: &Path, ctx: &ValidationContext) -> Vec<co::ValidationIssue> {
    let mut all_issues = Vec::new();

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                validate_context(&path, ctx, &mut all_issues);
            }
        }
    }

    all_issues
}

/// Validate all files in a context
fn validate_context(
    context_path: &Path,
    ctx: &ValidationContext,
    issues: &mut Vec<co::ValidationIssue>,
) {
    if let Ok(entries) = fs::read_dir(context_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                validate_directory(&path, ctx, issues);
            } else if path.is_file() && path.extension().map_or(false, |e| e == "md") {
                // Skip README files
                if path.file_name().map_or(false, |n| n == "README.md") {
                    continue;
                }
                let file_issues = co::validate::validate_file(&path, ctx);
                issues.extend(file_issues);
            }
        }
    }
}

/// Validate all files in a directory
fn validate_directory(
    dir: &Path,
    ctx: &ValidationContext,
    issues: &mut Vec<co::ValidationIssue>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == "md") {
                let file_issues = co::validate::validate_file(&path, ctx);
                issues.extend(file_issues);
            }
        }
    }
}

/// Count content files
fn count_content_files(root: &Path) -> usize {
    let mut count = 0;

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                count += count_files_in_context(&path);
            }
        }
    }

    count
}

/// Count files in a context
fn count_files_in_context(context_path: &Path) -> usize {
    let mut count = 0;

    if let Ok(entries) = fs::read_dir(context_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_files_in_directory(&path);
            } else if path.is_file()
                && path.extension().map_or(false, |e| e == "md")
                && path.file_name().map_or(true, |n| n != "README.md")
            {
                count += 1;
            }
        }
    }

    count
}

/// Count files in a directory
fn count_files_in_directory(dir: &Path) -> usize {
    let mut count = 0;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == "md") {
                count += 1;
            }
        }
    }

    count
}

/// Check if a path is hidden
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map_or(false, |n| n.starts_with('.'))
}
