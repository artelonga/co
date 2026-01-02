//! Persistent index for fast queries

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Entry in the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    /// File path relative to graph root
    pub path: PathBuf,

    /// Node type
    pub node_type: String,

    /// Node ID
    pub id: String,

    /// Content hash for change detection
    pub content_hash: u64,

    /// File modification time
    pub mtime: u64,

    /// Extracted frontmatter fields for querying (string values only)
    pub fields: HashMap<String, String>,

    /// Scope/context this entry belongs to (e.g., "en", "private")
    pub scope: String,

    /// Content language if specified in frontmatter
    pub language: Option<String>,
}

/// The persistent index
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Index {
    /// Version for migration
    pub version: u32,

    /// Entries by node ID
    pub entries: HashMap<String, IndexEntry>,
}

impl Index {
    /// Current index version
    pub const VERSION: u32 = 1;

    /// Create a new empty index
    pub fn new() -> Self {
        Index {
            version: Self::VERSION,
            entries: HashMap::new(),
        }
    }

    /// Add an entry to the index
    pub fn insert(&mut self, entry: IndexEntry) {
        self.entries.insert(entry.id.clone(), entry);
    }

    /// Get an entry by ID
    pub fn get(&self, id: &str) -> Option<&IndexEntry> {
        self.entries.get(id)
    }

    /// Remove an entry
    pub fn remove(&mut self, id: &str) -> Option<IndexEntry> {
        self.entries.remove(id)
    }

    /// Count entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize to bytes (bincode)
    pub fn to_bytes(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_entry(id: &str) -> IndexEntry {
        IndexEntry {
            path: PathBuf::from(format!("test/{}.md", id)),
            node_type: "task".to_string(),
            id: id.to_string(),
            content_hash: 12345,
            mtime: 1234567890,
            fields: HashMap::new(),
            scope: "en".to_string(),
            language: Some("english".to_string()),
        }
    }

    #[test]
    fn test_entry_with_scope_and_language() {
        let entry = IndexEntry {
            path: PathBuf::from("private/tasks/task-001.md"),
            node_type: "task".to_string(),
            id: "task-001".to_string(),
            content_hash: 12345,
            mtime: 1234567890,
            fields: HashMap::new(),
            scope: "private".to_string(),
            language: Some("portuguese".to_string()),
        };

        assert_eq!(entry.scope, "private");
        assert_eq!(entry.language, Some("portuguese".to_string()));
    }

    #[test]
    fn test_entry_without_language() {
        let entry = IndexEntry {
            path: PathBuf::from("en/definitions/math.md"),
            node_type: "definition".to_string(),
            id: "math".to_string(),
            content_hash: 99999,
            mtime: 1234567890,
            fields: HashMap::new(),
            scope: "en".to_string(),
            language: None,
        };

        assert_eq!(entry.scope, "en");
        assert!(entry.language.is_none());
    }

    #[test]
    fn test_new_index() {
        let index = Index::new();
        assert_eq!(index.version, Index::VERSION);
        assert!(index.is_empty());
    }

    #[test]
    fn test_insert_and_get() {
        let mut index = Index::new();
        let entry = create_test_entry("task-001");

        index.insert(entry);

        assert_eq!(index.len(), 1);
        assert!(index.get("task-001").is_some());
        assert!(index.get("nonexistent").is_none());
    }

    #[test]
    fn test_remove() {
        let mut index = Index::new();
        index.insert(create_test_entry("task-001"));

        let removed = index.remove("task-001");
        assert!(removed.is_some());
        assert!(index.is_empty());
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut index = Index::new();
        index.insert(create_test_entry("task-001"));
        index.insert(create_test_entry("task-002"));

        let bytes = index.to_bytes().unwrap();
        let restored = Index::from_bytes(&bytes).unwrap();

        assert_eq!(restored.version, index.version);
        assert_eq!(restored.len(), 2);
        assert!(restored.get("task-001").is_some());
        assert!(restored.get("task-002").is_some());
    }
}
