//! # CO Core
//!
//! Graph-based content management engine.
//!
//! CO is a graph database where:
//! - **Nodes** = definitions, tasks, projects
//! - **Edges** = relationships (inherits, translates_to, references)
//!
//! ## Architecture
//!
//! ```text
//! CO (root)
//! ├── Language (inherits CO)
//! │   ├── English
//! │   ├── Portuguese
//! │   └── ...
//! └── Domain (instantiates Language)
//!     └── Definitions, Tasks, Projects
//! ```
//!
//! ## Usage as Dependency
//!
//! ```toml
//! [dependencies]
//! co = { git = "https://github.com/institutional-pointset/co", branch = "main" }
//! ```
//!
//! ## Example
//!
//! ```rust,ignore
//! use co::{Graph, Language};
//!
//! let mut graph = Graph::new();
//! graph.add_language("english", "English");
//! graph.add_domain("my-project", "english");
//! graph.define("my-project", "sprint", "A fixed time period for work");
//! ```

pub mod agent;
pub mod archive;
pub mod config;
pub mod content;
pub mod edge;
pub mod entry;
pub mod feature;
pub mod frontmatter;
pub mod github;
pub mod graph;
pub mod index;
pub mod language;
pub mod mail;
pub mod manifest;
pub mod node;
pub mod payload;
pub mod query;
pub mod schema;
pub mod space;
pub mod storage;
pub mod sync;
pub mod types;
pub mod validate;
pub mod wikilink;

/// Generated protobuf types for Entry wire format.
pub mod proto {
    pub mod entry {
        include!(concat!(env!("OUT_DIR"), "/co.entry.rs"));
    }
}

// Re-export main types
pub use content::{ParsedContent, SectionSpec, specs_for_type};
pub use edge::{Edge, EdgeType};
pub use frontmatter::{BaseFrontmatter, Frontmatter};
pub use graph::Graph;
pub use index::{Index, IndexEntry};
pub use language::{I18n, Language, UiLabels};
pub use mail::{LogMailProvider, MailProvider};
pub use node::{Node, NodeId};
pub use query::Query;
pub use schema::{Definition, Domain, Project, Task};
pub use space::{Space, SpaceKind, SpaceLocation};
// Deprecated aliases for backwards compatibility
pub use entry::{
    Entry, FileStat, delete_entry, entry_to_markdown, move_entry, parse_entry_content, read_entry,
    scan_entries, split_frontmatter, write_entry, yaml_to_json,
};
#[allow(deprecated)]
pub use space::{Context, ContextKind, Scope, ScopeKind};
pub use storage::ContentStore;
pub use types::{
    Direction, ExegesisType, LanguageSpec, Lexicon, LexiconEntry, SemanticVersion, TypeKind,
    TypeRef,
};
pub use validate::{KNOWN_TYPES, Severity, ValidationContext, ValidationIssue};

// Feature system exports
pub use feature::schema::{FeatureSchema, PropertyDef, PropertyKind};
pub use feature::{Feature, FeatureRegistry};

// Config system exports
pub use config::{GlobalConfig, GroupDef, GroupsConfig, RepoConfig, RepoLocalConfig};

// GitHub integration exports
pub use github::{GhCli, GhError, GithubIssue, GithubLabel, GithubUser, MappedWorkItem};

/// The root type - CO itself
pub const CO_ROOT: &str = "co";

/// Supported languages (initial set)
pub const LANGUAGES: &[&str] = &["english", "portuguese", "guarani-mbya", "music", "math"];

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Characters explicitly forbidden in content IDs.
///
/// These characters are not allowed because they:
/// - Break filesystem paths: `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`
/// - Cause shell/parsing issues: `'`, `!`, `@`, `#`, `$`, `%`, `^`, `&`
/// - Are whitespace: space, tab, newline, carriage return
///
/// Allowed characters: alphanumeric (a-z, A-Z, 0-9), hyphen (`-`), dot (`.`), underscore (`_`)
pub const FORBIDDEN_ID_CHARS: &[char] = &[
    '/', '\\', ':', '*', '?', '"', '<', '>', '|', // Filesystem-unsafe
    '\'', '!', '@', '#', '$', '%', '^', '&', // Shell/special
    ' ', '\t', '\n', '\r', // Whitespace
];

/// Check if a character is valid for content IDs.
///
/// Valid characters are:
/// - Alphanumeric (a-z, A-Z, 0-9)
/// - Hyphen (`-`)
/// - Dot (`.`)
/// - Underscore (`_`)
#[inline]
pub fn is_valid_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '.' || c == '_'
}

/// Validate an ID string, returning any invalid characters found.
///
/// # Returns
/// - `Ok(())` if the ID contains only valid characters
/// - `Err(Vec<char>)` with the list of invalid characters found
///
/// # Example
/// ```
/// use co::validate_id;
///
/// assert!(validate_id("my-task-1").is_ok());
/// assert!(validate_id("task-37.1").is_ok());
/// assert!(validate_id("test/path").is_err());
/// ```
pub fn validate_id(id: &str) -> Result<(), Vec<char>> {
    let invalid: Vec<char> = id.chars().filter(|c| !is_valid_id_char(*c)).collect();
    if invalid.is_empty() {
        Ok(())
    } else {
        Err(invalid)
    }
}

#[cfg(test)]
mod id_tests {
    use super::*;

    #[test]
    fn test_forbidden_chars_are_explicit() {
        // Test ALL forbidden characters are rejected
        for c in FORBIDDEN_ID_CHARS {
            assert!(
                !is_valid_id_char(*c),
                "Character '{}' should be forbidden",
                c
            );
        }
    }

    #[test]
    fn test_valid_id_chars() {
        // Alphanumeric allowed
        assert!(is_valid_id_char('a'));
        assert!(is_valid_id_char('Z'));
        assert!(is_valid_id_char('5'));
        // Special allowed chars
        assert!(is_valid_id_char('-'));
        assert!(is_valid_id_char('.'));
        assert!(is_valid_id_char('_'));
    }

    #[test]
    fn test_validate_id_returns_invalid_chars() {
        let result = validate_id("test/path:name");
        assert!(result.is_err());
        let invalid = result.unwrap_err();
        assert!(invalid.contains(&'/'));
        assert!(invalid.contains(&':'));
    }

    #[test]
    fn test_validate_id_accepts_valid_id() {
        assert!(validate_id("my-task-1").is_ok());
        assert!(validate_id("task-37.1").is_ok());
        assert!(validate_id("user_story").is_ok());
    }
}
