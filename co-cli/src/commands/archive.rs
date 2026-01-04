//! Archive command - move content to archive (deindexed storage)
//!
//! Archives content by moving it to an archive directory within the same space,
//! adding `archived_at` timestamp and `indexed: false` to deindex from co.

use crate::i18n::load_i18n;
use chrono::Utc;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};

/// Archive action variants
pub enum ArchiveAction {
    /// Archive content (move to archive, add metadata)
    Archive { name: String, force: bool },
    /// Restore content from archive
    Restore { name: String },
    /// List archived items
    List,
}

/// Run the archive command
pub fn run(action: ArchiveAction) {
    match action {
        ArchiveAction::Archive { name, force } => archive_item(&name, force),
        ArchiveAction::Restore { name } => restore_item(&name),
        ArchiveAction::List => list_archived(),
    }
}

/// Archive a content item
fn archive_item(name: &str, force: bool) {
    let i18n = load_i18n();

    // Find the content file
    let file_path = match find_content_file(name) {
        Some(path) => path,
        None => {
            eprintln!(
                "{} {} '{}' not found",
                "error:".red().bold(),
                i18n.type_label("content"),
                name
            );
            std::process::exit(1);
        }
    };

    // Calculate archive path (insert "archive" after space directory)
    let archive_path = calculate_archive_path(&file_path);

    // Check if already archived
    if archive_path.exists() && !force {
        eprintln!(
            "{} '{}' already exists in archive",
            "error:".red().bold(),
            name
        );
        eprintln!("Use {} to replace with current version", "--force".cyan());
        std::process::exit(1);
    }

    // Read content and add archive metadata
    let content = match fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{} Failed to read {}: {}",
                "error:".red().bold(),
                file_path.display(),
                e
            );
            std::process::exit(1);
        }
    };

    let archived_content = add_archive_metadata(&content);

    // Create archive directory if needed
    if let Some(parent) = archive_path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!(
            "{} Failed to create archive directory: {}",
            "error:".red().bold(),
            e
        );
        std::process::exit(1);
    }

    // Write to archive location
    if let Err(e) = fs::write(&archive_path, archived_content) {
        eprintln!("{} Failed to write archive: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    // Remove original file
    if let Err(e) = fs::remove_file(&file_path) {
        eprintln!("{} Failed to remove original: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    // Output success message
    if force && archive_path.exists() {
        println!("{}", "Archived (replaced existing)".bold().green());
    } else {
        println!("{}", "Archived".bold().green());
    }
    println!("{}", "-".repeat(30));
    println!("  {}: {}", "from".cyan(), file_path.display());
    println!("  {}: {}", "to".cyan(), archive_path.display());
}

/// Restore a content item from archive
fn restore_item(name: &str) {
    let i18n = load_i18n();

    // Find the archived file
    let archive_path = match find_archived_file(name) {
        Some(path) => path,
        None => {
            eprintln!(
                "{} Archived {} '{}' not found",
                "error:".red().bold(),
                i18n.type_label("content"),
                name
            );
            std::process::exit(1);
        }
    };

    // Calculate original path (remove "archive" from path)
    let original_path = calculate_original_path(&archive_path);

    // Read content and remove archive metadata
    let content = match fs::read_to_string(&archive_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "{} Failed to read {}: {}",
                "error:".red().bold(),
                archive_path.display(),
                e
            );
            std::process::exit(1);
        }
    };

    let restored_content = remove_archive_metadata(&content);

    // Ensure original directory exists
    if let Some(parent) = original_path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!(
            "{} Failed to create directory: {}",
            "error:".red().bold(),
            e
        );
        std::process::exit(1);
    }

    // Write to original location
    if let Err(e) = fs::write(&original_path, restored_content) {
        eprintln!(
            "{} Failed to write restored file: {}",
            "error:".red().bold(),
            e
        );
        std::process::exit(1);
    }

    // Remove archive file
    if let Err(e) = fs::remove_file(&archive_path) {
        eprintln!("{} Failed to remove archive: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!("{}", "Restored".bold().green());
    println!("{}", "-".repeat(30));
    println!("  {}: {}", "from".cyan(), archive_path.display());
    println!("  {}: {}", "to".cyan(), original_path.display());
}

/// List all archived items
fn list_archived() {
    let archived_files = find_all_archived_files();

    if archived_files.is_empty() {
        println!("{}", "No archived items".yellow());
        return;
    }

    println!("{}", "Archived Items".bold());
    println!("{}", "-".repeat(40));

    for (name, path) in archived_files {
        println!(
            "  {} {}",
            name.cyan(),
            format!("({})", path.display()).dimmed()
        );
    }
}

/// Find a content file by name across all scopes (excluding archive directories)
fn find_content_file(name: &str) -> Option<PathBuf> {
    let current_dir = Path::new(".");

    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let dir_name = path.file_name()?.to_str()?;
                // Skip archive directories at top level
                if dir_name == "archive" {
                    continue;
                }

                // Search in type subdirectories (tasks/, definitions/, etc.)
                if let Ok(type_dirs) = fs::read_dir(&path) {
                    for type_dir in type_dirs.flatten() {
                        let type_path = type_dir.path();
                        // Skip archive subdirectories
                        if type_path
                            .file_name()
                            .map(|n| n == "archive")
                            .unwrap_or(false)
                        {
                            continue;
                        }
                        if type_path.is_dir() {
                            let file_path = type_path.join(format!("{}.md", name));
                            if file_path.exists() {
                                return Some(file_path);
                            }
                        }
                    }
                }
                // Check directly in the scope
                let direct_path = path.join(format!("{}.md", name));
                if direct_path.exists() {
                    return Some(direct_path);
                }
            }
        }
    }

    None
}

/// Find an archived file by name
fn find_archived_file(name: &str) -> Option<PathBuf> {
    let current_dir = Path::new(".");

    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Look for archive directory within each space
                let archive_dir = path.join("archive");
                if archive_dir.is_dir()
                    && let Some(found) = search_archive_dir(&archive_dir, name)
                {
                    return Some(found);
                }
            }
        }
    }

    None
}

/// Search within an archive directory for a file
fn search_archive_dir(archive_dir: &Path, name: &str) -> Option<PathBuf> {
    if let Ok(type_dirs) = fs::read_dir(archive_dir) {
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
    // Check directly in archive
    let direct_path = archive_dir.join(format!("{}.md", name));
    if direct_path.exists() {
        return Some(direct_path);
    }
    None
}

/// Find all archived files across all spaces
fn find_all_archived_files() -> Vec<(String, PathBuf)> {
    let mut archived = Vec::new();
    let current_dir = Path::new(".");

    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let archive_dir = path.join("archive");
                if archive_dir.is_dir() {
                    collect_archived_from_dir(&archive_dir, &mut archived);
                }
            }
        }
    }

    archived
}

/// Collect archived files from an archive directory
fn collect_archived_from_dir(archive_dir: &Path, archived: &mut Vec<(String, PathBuf)>) {
    if let Ok(type_dirs) = fs::read_dir(archive_dir) {
        for type_dir in type_dirs.flatten() {
            let type_path = type_dir.path();
            if type_path.is_dir() {
                if let Ok(files) = fs::read_dir(&type_path) {
                    for file in files.flatten() {
                        let file_path = file.path();
                        if file_path.extension().map(|e| e == "md").unwrap_or(false)
                            && let Some(stem) = file_path.file_stem()
                        {
                            archived.push((stem.to_string_lossy().to_string(), file_path));
                        }
                    }
                }
            } else if type_path.extension().map(|e| e == "md").unwrap_or(false) {
                // Direct files in archive
                if let Some(stem) = type_path.file_stem() {
                    archived.push((stem.to_string_lossy().to_string(), type_path));
                }
            }
        }
    }
}

/// Calculate archive path from original path
/// Example: work/tasks/task-1.md -> work/archive/tasks/task-1.md
fn calculate_archive_path(original: &Path) -> PathBuf {
    let components: Vec<_> = original.components().collect();

    // Find the space directory (first component after ".")
    // Pattern: ./space/type/file.md -> ./space/archive/type/file.md
    if components.len() >= 3 {
        let mut new_path = PathBuf::new();
        new_path.push(components[0]); // "."
        new_path.push(components[1]); // space dir
        new_path.push("archive");
        for comp in &components[2..] {
            new_path.push(comp);
        }
        return new_path;
    }

    // Fallback: just add archive prefix
    let mut new_path = PathBuf::from("archive");
    new_path.push(original);
    new_path
}

/// Calculate original path from archive path
/// Example: work/archive/tasks/task-1.md -> work/tasks/task-1.md
fn calculate_original_path(archive: &Path) -> PathBuf {
    let components: Vec<_> = archive.components().collect();
    let mut new_path = PathBuf::new();

    let mut skip_next = false;
    for (i, comp) in components.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }

        let comp_str = comp.as_os_str().to_string_lossy();
        if comp_str == "archive" && i > 0 {
            // Skip the "archive" component
            continue;
        }

        new_path.push(comp);
    }

    new_path
}

/// Add archive metadata to frontmatter
fn add_archive_metadata(content: &str) -> String {
    let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // Find the end of frontmatter
    if let Some(rest) = content.strip_prefix("---")
        && let Some(end) = rest.find("---")
    {
        let frontmatter = &rest[..end];
        let after = &rest[end..];

        // Add archived_at and indexed: false before closing ---
        return format!(
            "---{}archived_at: {}\nindexed: false\n{}",
            frontmatter, timestamp, after
        );
    }

    // No frontmatter found, add one
    format!(
        "---\narchived_at: {}\nindexed: false\n---\n\n{}",
        timestamp, content
    )
}

/// Remove archive metadata from frontmatter
fn remove_archive_metadata(content: &str) -> String {
    let mut result = String::new();
    let mut in_frontmatter = false;

    for line in content.lines() {
        if line == "---" {
            in_frontmatter = !in_frontmatter;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        // Skip archive-related fields in frontmatter
        if in_frontmatter
            && (line.starts_with("archived_at:") || line.starts_with("indexed: false"))
        {
            continue;
        }

        result.push_str(line);
        result.push('\n');
    }

    // Remove trailing newline if original didn't have one
    if !content.ends_with('\n') {
        result.pop();
    }

    result
}
