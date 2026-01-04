//! Space discovery and management
//!
//! Spaces are directories that form a hierarchical namespace for content.
//! They can be nested (e.g., `monica/`, `monica/en/`, `work/private/`).
//!
//! # Terminology
//!
//! - **Space** = Any namespace directory (hierarchical, fractal/recursive)
//! - **Context** = User-provided content/prompts only (kept separate)
//!
//! The term "scope" is deprecated and aliased to Space for backwards compatibility.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The type of a space detected from its README frontmatter
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpaceKind {
    /// A natural or formal language (e.g., english, math)
    Language,
    /// A user-defined space for content organization
    Space,
    /// Unknown or unrecognized type
    #[default]
    Unknown,
}

/// Backwards-compatible alias for SpaceKind
#[deprecated(since = "0.13.1", note = "Use SpaceKind instead")]
pub type ContextKind = SpaceKind;

/// Backwards-compatible alias for SpaceKind
#[deprecated(since = "0.13.1", note = "Use SpaceKind instead")]
pub type ScopeKind = SpaceKind;

/// Represents a discovered space in the repository
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    /// Unique identifier (directory name)
    pub id: String,
    /// Type of space (language or user-defined)
    pub space_kind: SpaceKind,
    /// Path to the space directory
    pub path: PathBuf,
    /// Whether this space is a git submodule
    pub is_submodule: bool,
}

/// Backwards-compatible alias for Space
#[deprecated(since = "0.13.1", note = "Use Space instead")]
pub type Context = Space;

/// Backwards-compatible alias for Space
#[deprecated(since = "0.13.1", note = "Use Space instead")]
pub type Scope = Space;

impl Space {
    /// Create a new space
    pub fn new(id: impl Into<String>, space_kind: SpaceKind, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            space_kind,
            path: path.into(),
            is_submodule: false,
        }
    }

    /// Create a language space
    pub fn language(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::new(id, SpaceKind::Language, path)
    }

    /// Create a user-defined space
    pub fn new_space(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::new(id, SpaceKind::Space, path)
    }

    /// Mark this space as a git submodule
    pub fn with_submodule(mut self, is_submodule: bool) -> Self {
        self.is_submodule = is_submodule;
        self
    }

    /// Check if this is a language space
    pub fn is_language(&self) -> bool {
        matches!(self.space_kind, SpaceKind::Language)
    }

    /// Check if this is a user-defined space
    pub fn is_space(&self) -> bool {
        matches!(self.space_kind, SpaceKind::Space)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_space_struct() {
        let space = Space::new("private", SpaceKind::Space, "/path/to/private");
        assert_eq!(space.id, "private");
        assert_eq!(space.space_kind, SpaceKind::Space);
        assert_eq!(space.path, PathBuf::from("/path/to/private"));
        assert!(!space.is_submodule);
    }

    #[test]
    fn test_space_language_constructor() {
        let lang = Space::language("en", "/path/to/en");
        assert!(lang.is_language());
        assert!(!lang.is_space());
    }

    #[test]
    fn test_space_with_submodule() {
        let space = Space::new_space("shared", "/path/to/shared").with_submodule(true);
        assert!(space.is_submodule);
    }

    #[test]
    fn test_space_kind_default() {
        let kind: SpaceKind = Default::default();
        assert_eq!(kind, SpaceKind::Unknown);
    }
}
