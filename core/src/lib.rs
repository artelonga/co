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

pub mod archive;
pub mod edge;
pub mod frontmatter;
pub mod graph;
pub mod index;
pub mod language;
pub mod node;
pub mod query;
pub mod schema;
pub mod scope;
pub mod storage;
pub mod types;
pub mod validate;

// Re-export main types
pub use edge::{Edge, EdgeType};
pub use frontmatter::{BaseFrontmatter, Frontmatter};
pub use graph::Graph;
pub use index::{Index, IndexEntry};
pub use language::{I18n, Language, UiLabels};
pub use node::{Node, NodeId};
pub use query::Query;
pub use schema::{Definition, Domain, Project, Task};
pub use scope::{Context, ContextKind};
pub use storage::ContentStore;
pub use types::{
    Direction, ExegesisType, LanguageSpec, Lexicon, LexiconEntry, SemanticVersion, TypeKind,
    TypeRef,
};
pub use validate::{Severity, ValidationContext, ValidationIssue, KNOWN_TYPES};

/// The root type - CO itself
pub const CO_ROOT: &str = "co";

/// Supported languages (initial set)
pub const LANGUAGES: &[&str] = &["english", "portuguese", "guarani-mbya", "music", "math"];

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
