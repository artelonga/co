//! Archive and compression

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// An archive of compressed content
#[derive(Debug, Serialize, Deserialize)]
pub struct Archive {
    /// Archive name (e.g., "2024-Q4")
    pub name: String,

    /// Manifest of archived entries
    pub manifest: HashMap<String, ArchiveEntry>,

    /// Path to the compressed data file
    pub data_path: PathBuf,
}

/// Entry in an archive manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
    /// Original file path
    pub path: PathBuf,

    /// Node ID
    pub id: String,

    /// Offset in the compressed archive
    pub offset: u64,

    /// Compressed size
    pub compressed_size: u64,

    /// Original size
    pub original_size: u64,

    /// Frontmatter (preserved for querying without decompression)
    pub frontmatter: serde_json::Value,
}

impl Archive {
    /// Create a new archive
    pub fn new(name: &str) -> Self {
        Archive {
            name: name.to_string(),
            manifest: HashMap::new(),
            data_path: PathBuf::new(),
        }
    }

    /// Add an entry to the manifest
    pub fn add_entry(&mut self, entry: ArchiveEntry) {
        self.manifest.insert(entry.id.clone(), entry);
    }

    /// Get an entry by ID
    pub fn get(&self, id: &str) -> Option<&ArchiveEntry> {
        self.manifest.get(id)
    }

    /// Count entries
    pub fn len(&self) -> usize {
        self.manifest.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.manifest.is_empty()
    }
}
