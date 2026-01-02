//! Schema types for CO content
//!
//! These types define the structure of definitions, tasks, projects, etc.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A definition (exegetic abstraction)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Definition {
    /// Unique identifier
    pub id: String,

    /// The symbol being defined
    pub symbol: String,

    /// The canonical (source) language
    pub canonical: String,

    /// Translations to all languages
    pub translations: HashMap<String, DefinitionTranslation>,
}

/// Translation of a definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionTranslation {
    /// The term in this language
    pub term: String,

    /// The definition in this language
    pub definition: String,

    /// An example in this language
    pub example: Option<String>,
}

/// A task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier
    pub id: String,

    /// Language of this task
    pub language: String,

    /// Title
    pub title: String,

    /// Status
    pub status: TaskStatus,

    /// Priority
    pub priority: Priority,

    /// Assignee
    pub assignee: Option<String>,

    /// Due date
    pub due: Option<DateTime<Utc>>,

    /// Tags
    pub tags: Vec<String>,

    /// Parent project
    pub project: Option<String>,

    /// Links to translations
    pub translations: Vec<String>,

    /// Created timestamp
    pub created: DateTime<Utc>,

    /// Updated timestamp
    pub updated: DateTime<Utc>,
}

/// Task status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    Todo,
    InProgress,
    Review,
    Done,
}

/// Priority level
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

/// A project (a domain created by a group with an objective)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Unique identifier
    pub id: String,

    /// Title
    pub title: String,

    /// Status
    pub status: ProjectStatus,

    /// Priority
    pub priority: Priority,

    /// Tags
    pub tags: Vec<String>,

    /// Created timestamp
    pub created: DateTime<Utc>,
}

/// Project status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Planning,
    Active,
    OnHold,
    Completed,
    Cancelled,
}

/// A domain (instantiation of a language with added vocabulary)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Domain {
    /// Unique identifier
    pub id: String,

    /// Display name
    pub name: String,

    /// Parent language this domain inherits from
    pub inherits: String,

    /// Domain-specific vocabulary
    pub vocabulary: HashMap<String, String>,
}
