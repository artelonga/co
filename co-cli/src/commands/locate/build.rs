//! Build index command - `co locate build`
//!
//! Scans all scopes and builds a persistent index at `.co/index.bin`.

use co::{Index, IndexEntry, ParsedContent, specs_for_type};
use colored::Colorize;
use serde::Deserialize;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
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

/// Run the build command
pub fn run() {
    println!("{}", "Building index...".bold());

    let mut index = Index::new();
    let current_dir = Path::new(".");
    let mut indexed_count = 0;

    // Iterate through all directories (potential scopes)
    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                let scope_name = path.file_name().unwrap().to_string_lossy().to_string();

                // Index files in type subdirectories (tasks/, definitions/, etc.)
                if let Ok(type_dirs) = fs::read_dir(&path) {
                    for type_dir in type_dirs.flatten() {
                        let type_path = type_dir.path();
                        if type_path.is_dir() {
                            indexed_count += index_directory(&type_path, &scope_name, &mut index);
                        }
                    }
                }

                // Also index files directly in the scope root
                indexed_count += index_directory(&path, &scope_name, &mut index);
            }
        }
    }

    // Ensure .co directory exists
    let co_dir = current_dir.join(".co");
    if !co_dir.exists()
        && let Err(e) = fs::create_dir_all(&co_dir)
    {
        eprintln!(
            "{} Failed to create .co directory: {}",
            "error:".red().bold(),
            e
        );
        std::process::exit(1);
    }

    // Write index to .co/index.bin
    let index_path = co_dir.join("index.bin");
    match index.to_bytes() {
        Ok(bytes) => {
            if let Err(e) = fs::write(&index_path, bytes) {
                eprintln!("{} Failed to write index: {}", "error:".red().bold(), e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("{} Failed to serialize index: {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    }

    println!(
        "{} Index built with {} entries",
        "success:".green().bold(),
        indexed_count.to_string().cyan()
    );
}

/// Index all markdown files in a directory
fn index_directory(dir: &Path, scope: &str, index: &mut Index) -> usize {
    let mut count = 0;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|e| e == "md")
                && let Some(entry) = create_index_entry(&path, scope)
            {
                index.insert(entry);
                count += 1;
            }
        }
    }

    count
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

    // Parse content sections for user-story and task types (EPIC 13)
    if let Some(specs) = specs_for_type(&node_type) {
        let body_start = end_idx + 7; // Skip past "\n---\n"
        if body_start < content.len() {
            let body = &content[body_start..];
            let parsed = ParsedContent::parse(body, &specs);

            // Add parsed sections to fields
            for (field_name, section_content) in parsed.sections {
                fields.insert(field_name, section_content);
            }

            // Add title if present
            if let Some(title) = parsed.title {
                fields.insert("title".to_string(), title);
            }
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
