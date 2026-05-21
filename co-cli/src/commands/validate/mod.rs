//! Validate command - content integrity validation
//!
//! Validates frontmatter fields and references for content integrity.
//!
//! Subcommands:
//! - `all` - Validate all content files
//! - `item` - Validate a specific file
//!
//! Examples:
//!   co validate all           # Validate all content
//!   co validate item my-task  # Validate specific item

pub mod all;
pub mod deploy;
pub mod item;
