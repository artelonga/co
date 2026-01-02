//! Node types for the CO graph
//!
//! Nodes are the fundamental units of the graph - they represent
//! definitions, concepts, tasks, projects, etc.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unique identifier for a node
pub type NodeId = String;

/// The type of a node in the graph
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeType {
    /// The root node (CO itself)
    Root,

    /// A language type (English, Portuguese, Math, etc.)
    Language,

    /// A domain/project that instantiates a language
    Domain,

    /// A definition (exegetic abstraction)
    Definition,

    /// A task
    Task,

    /// A project
    Project,

    /// Generic content
    Content,
}

/// A node in the CO graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Unique identifier
    pub id: NodeId,

    /// Type of this node
    pub node_type: NodeType,

    /// Human-readable name/title
    pub name: String,

    /// For definitions: the symbol being defined
    pub symbol: Option<String>,

    /// For definitions: the meaning
    pub meaning: Option<String>,

    /// Translations to other languages
    /// Key = language id, Value = translated content
    pub translations: HashMap<String, Translation>,

    /// File path if loaded from disk
    pub file_path: Option<String>,

    /// Additional metadata
    pub metadata: HashMap<String, serde_yaml::Value>,
}

/// A translation of a node's content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Translation {
    /// The term in this language
    pub term: String,

    /// The definition in this language
    pub definition: Option<String>,

    /// An example in this language
    pub example: Option<String>,
}

impl Node {
    /// Create a new node
    pub fn new(id: NodeId, node_type: NodeType, name: String) -> Self {
        Node {
            id,
            node_type,
            name,
            symbol: None,
            meaning: None,
            translations: HashMap::new(),
            file_path: None,
            metadata: HashMap::new(),
        }
    }

    /// Create a definition node
    pub fn definition(id: NodeId, symbol: String, meaning: String) -> Self {
        Node {
            id,
            node_type: NodeType::Definition,
            name: symbol.clone(),
            symbol: Some(symbol),
            meaning: Some(meaning),
            translations: HashMap::new(),
            file_path: None,
            metadata: HashMap::new(),
        }
    }

    /// Create a task node
    pub fn task(id: NodeId, title: String) -> Self {
        Node {
            id,
            node_type: NodeType::Task,
            name: title,
            symbol: None,
            meaning: None,
            translations: HashMap::new(),
            file_path: None,
            metadata: HashMap::new(),
        }
    }

    /// Add a translation
    pub fn add_translation(&mut self, language: &str, term: &str, definition: Option<&str>) {
        self.translations.insert(
            language.to_string(),
            Translation {
                term: term.to_string(),
                definition: definition.map(String::from),
                example: None,
            },
        );
    }

    /// Get translation for a language
    pub fn translation(&self, language: &str) -> Option<&Translation> {
        self.translations.get(language)
    }

    /// Check if this node is a language
    pub fn is_language(&self) -> bool {
        matches!(self.node_type, NodeType::Language)
    }

    /// Check if this node is a definition
    pub fn is_definition(&self) -> bool {
        matches!(self.node_type, NodeType::Definition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_definition() {
        let node = Node::definition(
            "def-sprint".into(),
            "sprint".into(),
            "A fixed time period".into(),
        );

        assert_eq!(node.symbol, Some("sprint".into()));
        assert!(node.is_definition());
    }

    #[test]
    fn test_add_translation() {
        let mut node = Node::definition("def-hello".into(), "hello".into(), "A greeting".into());

        node.add_translation("portuguese", "olá", Some("Uma saudação"));

        let pt = node.translation("portuguese").unwrap();
        assert_eq!(pt.term, "olá");
    }
}
