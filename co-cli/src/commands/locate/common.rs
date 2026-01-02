//! Common utilities for locate subsystem

use std::path::Path;

/// Check if a path is hidden (starts with .)
pub fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map_or(false, |n| n.starts_with('.'))
}
