//! Update index command - `co locate update`
//!
//! Incrementally updates the index by checking file hashes.

use co::{Index, IndexEntry};
use colored::Colorize;
use serde::Deserialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::SystemTime;

/// Frontmatter for indexing
#[derive(Debug, Deserialize)]
struct IndexableFrontmatter {
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(flatten)]
    fields: HashMap<String, serde_yaml::Value>,
}

/// Run the update command
pub fn run() {
    println!("{}", "Updating index...".bold());

    let index_path = Path::new(".co/index.bin");

    // Load existing index or create new
    let mut index = if index_path.exists() {
        match fs::read(index_path) {
            Ok(bytes) => match Index::from_bytes(&bytes) {
                Ok(idx) => idx,
                Err(_) => {
                    println!("{}", "Index corrupted, rebuilding...".yellow());
                    Index::new()
                }
            },
            Err(_) => Index::new(),
        }
    } else {
        println!("{}", "No index found, building from scratch...".yellow());
        Index::new()
    };

    let current_dir = Path::new(".");
    let mut updated_count = 0;
    let mut added_count = 0;
    let mut removed_count = 0;
    let mut seen_ids: HashSet<String> = HashSet::new();

    // Iterate through all directories (potential scopes)
    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                let scope_name = path.file_name().unwrap().to_string_lossy().to_string();

                // Index files in type subdirectories
                if let Ok(type_dirs) = fs::read_dir(&path) {
                    for type_dir in type_dirs.flatten() {
                        let type_path = type_dir.path();
                        if type_path.is_dir() {
                            let (u, a) = update_directory(
                                &type_path,
                                &scope_name,
                                &mut index,
                                &mut seen_ids,
                            );
                            updated_count += u;
                            added_count += a;
                        }
                    }
                }

                // Also index files directly in the scope root
                let (u, a) = update_directory(&path, &scope_name, &mut index, &mut seen_ids);
                updated_count += u;
                added_count += a;
            }
        }
    }

    // Remove entries for files that no longer exist
    let to_remove: Vec<String> = index
        .entries
        .keys()
        .filter(|id| !seen_ids.contains(*id))
        .cloned()
        .collect();

    for id in &to_remove {
        index.remove(id);
        removed_count += 1;
    }

    // Ensure .co directory exists
    let co_dir = current_dir.join(".co");
    if !co_dir.exists() {
        if let Err(e) = fs::create_dir_all(&co_dir) {
            eprintln!(
                "{} Failed to create .co directory: {}",
                "error:".red().bold(),
                e
            );
            std::process::exit(1);
        }
    }

    // Write index
    match index.to_bytes() {
        Ok(bytes) => {
            if let Err(e) = fs::write(index_path, bytes) {
                eprintln!("{} Failed to write index: {}", "error:".red().bold(), e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("{} Failed to serialize index: {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    }

    // Summary
    if updated_count == 0 && added_count == 0 && removed_count == 0 {
        println!("{} Index is up to date", "success:".green().bold());
    } else {
        if added_count > 0 {
            println!(
                "  {} {} added",
                added_count.to_string().cyan(),
                if added_count == 1 { "entry" } else { "entries" }
            );
        }
        if updated_count > 0 {
            println!(
                "  {} {} updated",
                updated_count.to_string().cyan(),
                if updated_count == 1 {
                    "entry"
                } else {
                    "entries"
                }
            );
        }
        if removed_count > 0 {
            println!(
                "  {} {} removed",
                removed_count.to_string().cyan(),
                if removed_count == 1 {
                    "entry"
                } else {
                    "entries"
                }
            );
        }
        println!("{} Index updated successfully", "success:".green().bold());
    }
}

/// Update index entries for all markdown files in a directory
/// Returns (updated_count, added_count)
fn update_directory(
    dir: &Path,
    scope: &str,
    index: &mut Index,
    seen_ids: &mut HashSet<String>,
) -> (usize, usize) {
    let mut updated = 0;
    let mut added = 0;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "md") {
                if let Some(new_entry) = create_index_entry(&path, scope) {
                    seen_ids.insert(new_entry.id.clone());

                    // Check if entry exists and has changed
                    if let Some(existing) = index.get(&new_entry.id) {
                        if existing.content_hash != new_entry.content_hash {
                            index.insert(new_entry);
                            updated += 1;
                        }
                    } else {
                        index.insert(new_entry);
                        added += 1;
                    }
                }
            }
        }
    }

    (updated, added)
}

/// Create an index entry from a markdown file
fn create_index_entry(path: &Path, scope: &str) -> Option<IndexEntry> {
    let content = fs::read_to_string(path).ok()?;

    // Extract frontmatter
    if !content.starts_with("---") {
        return None;
    }

    let end_idx = content[3..].find("\n---")?;
    let yaml_str = &content[4..end_idx + 3];

    let frontmatter: IndexableFrontmatter = serde_yaml::from_str(yaml_str).ok()?;

    // Get ID from frontmatter or filename
    let id = frontmatter.id.unwrap_or_else(|| {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });

    // Get node type from frontmatter
    let node_type = frontmatter.r#type.unwrap_or_else(|| "unknown".to_string());

    // Calculate content hash
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    let content_hash = hasher.finish();

    // Get modification time
    let mtime = fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Convert frontmatter fields to strings for querying
    let mut fields = HashMap::new();
    for (key, value) in frontmatter.fields {
        if let Some(s) = value.as_str() {
            fields.insert(key, s.to_string());
        } else if let Some(b) = value.as_bool() {
            fields.insert(key, b.to_string());
        } else if let Some(i) = value.as_i64() {
            fields.insert(key, i.to_string());
        } else if let Some(f) = value.as_f64() {
            fields.insert(key, f.to_string());
        }
    }

    Some(IndexEntry {
        path: path.to_path_buf(),
        node_type,
        id,
        content_hash,
        mtime,
        fields,
        scope: scope.to_string(),
        language: frontmatter.language,
    })
}

/// Check if a path is hidden (starts with .)
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}
