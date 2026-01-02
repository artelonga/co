//! Context discovery and management
//!
//! Contexts are directories at the repository root that contain content.
//! All .md files within a context become lexicon entries.
//!
//! # Terminology
//!
//! "Context" is used instead of "scope" for better interoperability with
//! agentic systems. A context provides the semantic frame for content.
//!
//! # Aliasing
//!
//! The term "scope" can be aliased to "context" if needed for backwards
//! compatibility. In code, prefer using `Context` and `ContextKind`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The type of a context detected from its README frontmatter
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextKind {
    /// A natural or formal language (e.g., english, math)
    Language,
    /// A user-defined context for content organization
    Context,
    /// Unknown or unrecognized type
    Unknown,
}

impl Default for ContextKind {
    fn default() -> Self {
        ContextKind::Unknown
    }
}

/// Backwards-compatible alias for ContextKind
#[deprecated(since = "0.2.0", note = "Use ContextKind instead")]
pub type ScopeKind = ContextKind;

/// Represents a discovered context in the repository
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// Unique identifier (directory name)
    pub id: String,
    /// Type of context (language or user-defined)
    pub context_type: ContextKind,
    /// Path to the context directory
    pub path: PathBuf,
    /// Whether this context is a git submodule
    pub is_submodule: bool,
}

/// Backwards-compatible alias for Context
#[deprecated(since = "0.2.0", note = "Use Context instead")]
pub type Scope = Context;

impl Context {
    /// Create a new context
    pub fn new(id: impl Into<String>, context_type: ContextKind, path: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            context_type,
            path: path.into(),
            is_submodule: false,
        }
    }

    /// Create a language context
    pub fn language(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::new(id, ContextKind::Language, path)
    }

    /// Create a user-defined context
    pub fn context(id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::new(id, ContextKind::Context, path)
    }

    /// Mark this context as a git submodule
    pub fn with_submodule(mut self, is_submodule: bool) -> Self {
        self.is_submodule = is_submodule;
        self
    }

    /// Check if this is a language context
    pub fn is_language(&self) -> bool {
        matches!(self.context_type, ContextKind::Language)
    }

    /// Check if this is a user-defined context
    pub fn is_context(&self) -> bool {
        matches!(self.context_type, ContextKind::Context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_struct() {
        let ctx = Context::new("private", ContextKind::Context, "/path/to/private");
        assert_eq!(ctx.id, "private");
        assert_eq!(ctx.context_type, ContextKind::Context);
        assert_eq!(ctx.path, PathBuf::from("/path/to/private"));
        assert!(!ctx.is_submodule);
    }

    #[test]
    fn test_context_language_constructor() {
        let lang = Context::language("en", "/path/to/en");
        assert!(lang.is_language());
        assert!(!lang.is_context());
    }

    #[test]
    fn test_context_with_submodule() {
        let ctx = Context::context("shared", "/path/to/shared").with_submodule(true);
        assert!(ctx.is_submodule);
    }

    #[test]
    fn test_context_kind_default() {
        let kind: ContextKind = Default::default();
        assert_eq!(kind, ContextKind::Unknown);
    }
}
