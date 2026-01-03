//! Content storage with memory-mapped file access

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Storage errors
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("File not found: {0}")]
    NotFound(PathBuf),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid content: {0}")]
    Invalid(String),
}

/// Content store - manages access to graph files
pub struct ContentStore {
    /// Root directory of the graph
    root: PathBuf,
}

impl ContentStore {
    /// Create a new content store
    pub fn new(root: impl AsRef<Path>) -> Self {
        ContentStore {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Get the root path
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Check if a file exists
    pub fn exists(&self, path: impl AsRef<Path>) -> bool {
        self.root.join(path).exists()
    }

    /// Read file contents
    pub fn read(&self, path: impl AsRef<Path>) -> Result<String, StorageError> {
        let full_path = self.root.join(path);
        std::fs::read_to_string(&full_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StorageError::NotFound(full_path)
            } else {
                StorageError::Io(e)
            }
        })
    }

    /// Write file contents atomically
    pub fn write(&self, path: impl AsRef<Path>, content: &str) -> Result<(), StorageError> {
        let full_path = self.root.join(path.as_ref());

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Write to temp file first, then rename (atomic)
        let temp_path = full_path.with_extension("tmp");
        std::fs::write(&temp_path, content)?;
        std::fs::rename(&temp_path, &full_path)?;

        Ok(())
    }

    /// List all files matching a pattern
    pub fn list(&self, pattern: &str) -> Result<Vec<PathBuf>, StorageError> {
        let mut files = Vec::new();

        for entry in walkdir::WalkDir::new(&self.root)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if (name.ends_with(pattern) || pattern == "*")
                && let Ok(rel) = path.strip_prefix(&self.root)
            {
                files.push(rel.to_path_buf());
            }
        }

        Ok(files)
    }
}
