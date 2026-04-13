//! Entry abstraction — every entity in a CO universe is an Entry.
//!
//! An Entry is a markdown file with YAML frontmatter:
//! - `.md` files are the **source of truth** (human-readable, Git-diffable, Obsidian-native)
//! - SQLite **indexes** entries for fast queries (disposable — rebuilt from files)
//! - Protobuf **encodes** entries on the wire (compact binary, gRPC-ready)
//!
//! # File Format
//!
//! ```markdown
//! ---
//! type: task
//! id: 1
//! title: "My task"
//! created: 2026-04-06T00:00:00Z
//! modified: 2026-04-06T12:00:00Z
//! tags: [mvp, backend]
//! ---
//!
//! Optional markdown body — the description.
//! Supports [[wikilinks]], #inline-tags, and all GFM.
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

/// File system metadata for an Entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileStat {
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub size: u64,
}

/// Universal entity — every content item in a CO universe.
///
/// An Entry is backed by a `.md` file with YAML frontmatter. The `entry_type`
/// comes from the `type` field in frontmatter.  The `frontmatter` holds all
/// structured metadata; `body` holds the free-form markdown body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Vault-relative path (e.g., `projects/MP/1.md`)
    pub path: String,
    /// Entity type from frontmatter `type` field (task, project, event, etc.)
    pub entry_type: String,
    /// Full frontmatter serialized to JSON (enables SQL json_extract queries)
    pub frontmatter: serde_json::Value,
    /// Markdown body (everything after the closing `---`)
    pub body: String,
    /// SHA-256 hex of body for change detection
    pub body_hash: String,
    /// File system metadata
    pub stat: FileStat,
}

impl Entry {
    /// Parse YAML frontmatter from a markdown string.
    /// Returns `Some((frontmatter_json, body_string))` or `None` if no frontmatter.
    pub fn parse_frontmatter(content: &str) -> Option<(serde_json::Value, String)> {
        let trimmed = content.trim_start();
        if !trimmed.starts_with("---") {
            return None;
        }
        let after_first = &trimmed[3..];
        let end = after_first.find("\n---")?;
        let yaml_str = &after_first[..end];
        let body = after_first[end + 4..].trim_start_matches('\n').to_string();
        let fm: serde_json::Value = serde_yaml::from_str(yaml_str).ok()?;
        Some((fm, body))
    }

    /// Compute the SHA-256 hash of a body string.
    pub fn hash_body(body: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(body.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Return the `title` from frontmatter, or an empty string.
    pub fn title(&self) -> &str {
        self.frontmatter
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    /// Return the `tags` array from frontmatter (Obsidian-compatible labels).
    pub fn tags(&self) -> Vec<String> {
        self.frontmatter
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Return the `parent` id (numeric or wikilink string) from frontmatter.
    pub fn parent(&self) -> Option<&serde_json::Value> {
        self.frontmatter.get("parent")
    }
}

// ---------------------------------------------------------------------------
// Frontmatter parsing
// ---------------------------------------------------------------------------

/// Split a markdown string into `(frontmatter_yaml, body)`.
///
/// Returns `("", full_content)` when no `---` fences are found.
pub fn split_frontmatter(content: &str) -> anyhow::Result<(String, String)> {
    if let Some(rest) = content.strip_prefix("---") {
        // Look for the closing fence — must be on its own line
        if let Some(end) = rest.find("\n---") {
            let fm = rest[..end].trim().to_string();
            let body = rest[end + 4..].trim_start_matches('\n').to_string();
            return Ok((fm, body));
        }
    }
    // No frontmatter — the entire file is the body
    Ok((String::new(), content.to_string()))
}

/// Convert a `serde_yaml::Value` into a `serde_json::Value`.
pub fn yaml_to_json(yaml: serde_yaml::Value) -> serde_json::Value {
    match yaml {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => serde_json::Value::Bool(b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::json!(f)
            } else {
                serde_json::Value::Null
            }
        }
        serde_yaml::Value::String(s) => serde_json::Value::String(s),
        serde_yaml::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.into_iter().map(yaml_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .into_iter()
                .filter_map(|(k, v)| k.as_str().map(|key| (key.to_string(), yaml_to_json(v))))
                .collect();
            serde_json::Value::Object(obj)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(tagged.value.clone()),
    }
}

/// Convert a `serde_json::Value` into a `serde_yaml::Value`.
pub fn json_to_yaml(json: serde_json::Value) -> serde_yaml::Value {
    match json {
        serde_json::Value::Null => serde_yaml::Value::Null,
        serde_json::Value::Bool(b) => serde_yaml::Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_yaml::Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_yaml::Value::Number(serde_yaml::Number::from(f))
            } else {
                serde_yaml::Value::Null
            }
        }
        serde_json::Value::String(s) => serde_yaml::Value::String(s),
        serde_json::Value::Array(arr) => {
            serde_yaml::Value::Sequence(arr.into_iter().map(json_to_yaml).collect())
        }
        serde_json::Value::Object(obj) => {
            let mapping: serde_yaml::Mapping = obj
                .into_iter()
                .map(|(k, v)| (serde_yaml::Value::String(k), json_to_yaml(v)))
                .collect();
            serde_yaml::Value::Mapping(mapping)
        }
    }
}

/// Parse raw markdown file content into an `Entry`.
pub fn parse_entry_content(path: &str, content: &str, stat: FileStat) -> anyhow::Result<Entry> {
    let (frontmatter_str, body) = split_frontmatter(content)?;

    let frontmatter: serde_json::Value = if frontmatter_str.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        let yaml: serde_yaml::Value = serde_yaml::from_str(&frontmatter_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse frontmatter in {}: {}", path, e))?;
        yaml_to_json(yaml)
    };

    let entry_type = frontmatter
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let body_hash = Entry::hash_body(&body);

    Ok(Entry {
        path: path.to_string(),
        entry_type,
        frontmatter,
        body: body.to_string(),
        body_hash,
        stat,
    })
}

/// Serialize an `Entry`'s frontmatter and body into a markdown string.
pub fn entry_to_markdown(frontmatter: &serde_json::Value, body: &str) -> anyhow::Result<String> {
    let yaml_val = json_to_yaml(frontmatter.clone());
    let yaml_str = serde_yaml::to_string(&yaml_val)
        .map_err(|e| anyhow::anyhow!("YAML serialization error: {}", e))?;
    Ok(format!("---\n{}---\n\n{}", yaml_str, body))
}

// ---------------------------------------------------------------------------
// File system helpers
// ---------------------------------------------------------------------------

fn file_stat(path: &Path) -> anyhow::Result<FileStat> {
    let meta = std::fs::metadata(path)?;
    let modified = meta
        .modified()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now());
    let created = meta
        .created()
        .map(DateTime::<Utc>::from)
        .unwrap_or(modified);
    Ok(FileStat {
        created,
        modified,
        size: meta.len(),
    })
}

/// Read and parse an entry from disk.
///
/// `universe_root` is the universe's root directory;
/// `path` is vault-relative (e.g., `"projects/MP/1.md"`).
pub fn read_entry(universe_root: &Path, path: &str) -> anyhow::Result<Entry> {
    let full_path = universe_root.join(path);
    let content = std::fs::read_to_string(&full_path)?;
    let stat = file_stat(&full_path)?;
    parse_entry_content(path, &content, stat)
}

/// Atomically write an entry to disk (temp file + rename).
///
/// Creates parent directories as needed.
pub fn write_entry(universe_root: &Path, entry: &Entry) -> anyhow::Result<()> {
    let full_path = universe_root.join(&entry.path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = entry_to_markdown(&entry.frontmatter, &entry.body)?;
    // Atomic write: write to a temp file then rename
    let tmp_path = full_path.with_extension("md.tmp");
    std::fs::write(&tmp_path, &content)?;
    std::fs::rename(&tmp_path, &full_path)?;
    Ok(())
}

/// Delete an entry file from disk.
pub fn delete_entry(universe_root: &Path, path: &str) -> anyhow::Result<()> {
    let full_path = universe_root.join(path);
    std::fs::remove_file(&full_path)?;
    Ok(())
}

/// Move/rename an entry file, updating its vault-relative path.
pub fn move_entry(universe_root: &Path, old_path: &str, new_path: &str) -> anyhow::Result<()> {
    let old_full = universe_root.join(old_path);
    let new_full = universe_root.join(new_path);
    if let Some(parent) = new_full.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&old_full, &new_full)?;
    Ok(())
}

/// Recursively scan all `.md` files under `universe_root` and parse them.
///
/// Files that fail to parse are skipped silently (returns partial results).
pub fn scan_entries(universe_root: &Path) -> anyhow::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for de in walkdir::WalkDir::new(universe_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
    {
        let rel_path = de
            .path()
            .strip_prefix(universe_root)
            .unwrap_or(de.path())
            .to_string_lossy()
            .replace('\\', "/");

        if let Ok(entry) = read_entry(universe_root, &rel_path) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    // --- Frontmatter round-trip ---

    fn make_stat() -> FileStat {
        FileStat {
            created: Utc::now(),
            modified: Utc::now(),
            size: 0,
        }
    }

    #[test]
    fn test_split_frontmatter_basic() {
        let content = "---\ntype: task\ntitle: Hello\n---\n\nBody text";
        let (fm, body) = split_frontmatter(content).unwrap();
        assert_eq!(fm, "type: task\ntitle: Hello");
        assert_eq!(body, "Body text");
    }

    #[test]
    fn test_split_frontmatter_no_fence() {
        let content = "Plain body, no frontmatter";
        let (fm, body) = split_frontmatter(content).unwrap();
        assert!(fm.is_empty());
        assert_eq!(body, content);
    }

    #[test]
    fn test_parse_entry_content_task() {
        let content = "---\ntype: task\nid: 1\ntitle: My Task\nstatus: todo\npriority: high\ntags:\n  - mvp\n  - backend\n---\n\nDescription here.";
        let stat = make_stat();
        let entry = parse_entry_content("projects/MP/1.md", content, stat).unwrap();
        assert_eq!(entry.entry_type, "task");
        assert_eq!(entry.title(), "My Task");
        assert_eq!(entry.tags(), vec!["mvp", "backend"]);
        assert_eq!(entry.body, "Description here.");
        assert_eq!(entry.path, "projects/MP/1.md");
    }

    #[test]
    fn test_entry_to_markdown_roundtrip() {
        let fm = json!({
            "type": "task",
            "id": 1,
            "title": "Round-trip test",
            "status": "todo",
            "tags": ["a", "b"]
        });
        let body = "Description body.";
        let md = entry_to_markdown(&fm, body).unwrap();
        assert!(md.starts_with("---\n"));
        assert!(md.contains("type: task"));
        assert!(md.contains("Round-trip test"));
        assert!(md.contains("Description body."));

        // Parse back
        let stat = make_stat();
        let entry = parse_entry_content("test.md", &md, stat).unwrap();
        assert_eq!(entry.entry_type, "task");
        assert_eq!(entry.title(), "Round-trip test");
        assert_eq!(entry.body, body);
    }

    // --- Type-specific frontmatter round-trips ---

    #[test]
    fn test_event_frontmatter() {
        let content = "---\ntype: event\nid: reunion-semanal\ntitle: Reunião Semanal\ndate: 2026-04-10\ntime: \"14:00\"\nlocation: Online\nstatus: scheduled\norganizer: yuri\ntags: []\n---\n\nAgenda items.";
        let stat = make_stat();
        let entry = parse_entry_content("events/reunion-semanal.md", content, stat).unwrap();
        assert_eq!(entry.entry_type, "event");
        assert_eq!(entry.frontmatter["status"], json!("scheduled"));
        assert_eq!(entry.frontmatter["location"], json!("Online"));
    }

    #[test]
    fn test_message_frontmatter() {
        let content = "---\ntype: message\nid: 1\ntitle: Release Notes\nfrom: yuri\nto: team\nthread: release-planning\ntags: []\n---\n\nHello team!";
        let stat = make_stat();
        let entry = parse_entry_content("messages/2026-04-06-release.md", content, stat).unwrap();
        assert_eq!(entry.entry_type, "message");
        assert_eq!(entry.frontmatter["thread"], json!("release-planning"));
    }

    #[test]
    fn test_comment_frontmatter() {
        let content = "---\ntype: comment\nid: 1\ntitle: ''\ntask: 21\nauthor: yuri\ntags: []\n---\n\nGreat work!";
        let stat = make_stat();
        let entry = parse_entry_content("projects/MP/comments/21-1.md", content, stat).unwrap();
        assert_eq!(entry.entry_type, "comment");
        assert_eq!(entry.frontmatter["task"], json!(21));
        assert_eq!(entry.frontmatter["author"], json!("yuri"));
    }

    #[test]
    fn test_page_frontmatter() {
        let content = "---\ntype: page\nid: sobre\ntitle: Sobre\nslug: sobre\nparent: index\norder: 1\ntags: []\n---\n\nPage content.";
        let stat = make_stat();
        let entry = parse_entry_content("content/sobre.md", content, stat).unwrap();
        assert_eq!(entry.entry_type, "page");
        assert_eq!(entry.frontmatter["slug"], json!("sobre"));
        assert_eq!(entry.frontmatter["order"], json!(1));
    }

    #[test]
    fn test_clip_frontmatter() {
        let content = "---\ntype: clip\nid: artigo-interessante\ntitle: Artigo Interessante\nsource: \"https://example.com/article\"\nauthor: \"Author Name\"\npublished: 2026-04-06\ntags: [reading]\n---\n\nClipped content.";
        let stat = make_stat();
        let entry = parse_entry_content("clips/artigo-interessante.md", content, stat).unwrap();
        assert_eq!(entry.entry_type, "clip");
        assert_eq!(entry.tags(), vec!["reading"]);
    }

    // --- File I/O round-trip ---

    #[test]
    fn test_write_and_read_entry() {
        let dir = tempdir().unwrap();
        let fm = json!({
            "type": "task",
            "id": 42,
            "title": "File I/O test",
            "status": "todo",
            "priority": "medium",
            "project": "TEST",
            "tags": ["test"]
        });
        let body = "This is the task description.";
        let stat = FileStat {
            created: Utc::now(),
            modified: Utc::now(),
            size: 0,
        };
        let entry = Entry {
            path: "projects/TEST/42.md".to_string(),
            entry_type: "task".to_string(),
            frontmatter: fm.clone(),
            body: body.to_string(),
            body_hash: Entry::hash_body(body),
            stat,
        };

        write_entry(dir.path(), &entry).unwrap();
        assert!(dir.path().join("projects/TEST/42.md").exists());

        let read_back = read_entry(dir.path(), "projects/TEST/42.md").unwrap();
        assert_eq!(read_back.entry_type, "task");
        assert_eq!(read_back.title(), "File I/O test");
        assert_eq!(read_back.body, body);
    }

    #[test]
    fn test_delete_entry() {
        let dir = tempdir().unwrap();
        let path = "test/delete-me.md";
        std::fs::create_dir_all(dir.path().join("test")).unwrap();
        std::fs::write(
            dir.path().join(path),
            "---\ntype: page\ntitle: Delete\n---\n\n",
        )
        .unwrap();
        assert!(dir.path().join(path).exists());
        delete_entry(dir.path(), path).unwrap();
        assert!(!dir.path().join(path).exists());
    }

    #[test]
    fn test_move_entry() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/file.md"),
            "---\ntype: page\ntitle: Move Test\n---\n\n",
        )
        .unwrap();
        move_entry(dir.path(), "src/file.md", "dest/file.md").unwrap();
        assert!(!dir.path().join("src/file.md").exists());
        assert!(dir.path().join("dest/file.md").exists());
    }

    #[test]
    fn test_scan_entries() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("projects/TEST")).unwrap();
        std::fs::write(
            dir.path().join("projects/TEST/1.md"),
            "---\ntype: task\nid: 1\ntitle: Task One\n---\n\nDesc 1",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("projects/TEST/2.md"),
            "---\ntype: task\nid: 2\ntitle: Task Two\n---\n\nDesc 2",
        )
        .unwrap();

        let entries = scan_entries(dir.path()).unwrap();
        assert_eq!(entries.len(), 2);
        let titles: Vec<&str> = entries.iter().map(|e| e.title()).collect();
        assert!(titles.contains(&"Task One"));
        assert!(titles.contains(&"Task Two"));
    }

    // --- Hash ---

    #[test]
    fn test_body_hash_deterministic() {
        let body = "Hello, world!";
        let h1 = Entry::hash_body(body);
        let h2 = Entry::hash_body(body);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn test_body_hash_different_bodies() {
        assert_ne!(Entry::hash_body("foo"), Entry::hash_body("bar"));
    }
}
