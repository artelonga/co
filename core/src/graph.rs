//! Graph database primitives
//!
//! CO is fundamentally a graph where nodes are definitions
//! and edges are relationships between them.

use crate::edge::{Edge, EdgeType};
use crate::node::{Node, NodeId};
use crate::types::{LanguageSpec, TypeKind};
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

/// The CO graph - a directed graph of nodes and edges
#[derive(Debug)]
pub struct Graph {
    /// The underlying petgraph structure
    inner: DiGraph<Node, Edge>,

    /// Map from NodeId to NodeIndex for fast lookup
    index: HashMap<NodeId, NodeIndex>,

    /// The root node (CO itself)
    root: Option<NodeIndex>,
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    /// Create a new empty graph
    pub fn new() -> Self {
        let mut graph = Graph {
            inner: DiGraph::new(),
            index: HashMap::new(),
            root: None,
        };

        // Initialize with CO root node
        graph.init_root();
        graph
    }

    /// Initialize the CO root node
    fn init_root(&mut self) {
        let root = Node::new(
            "co".into(),
            TypeKind::Root,
            "CO - the root supertype".into(),
        );
        let idx = self.inner.add_node(root);
        self.index.insert("co".into(), idx);
        self.root = Some(idx);
    }

    /// Get the root node
    pub fn root(&self) -> Option<&Node> {
        self.root.map(|idx| &self.inner[idx])
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: Node) -> NodeIndex {
        let id = node.id.clone();
        let idx = self.inner.add_node(node);
        self.index.insert(id, idx);
        idx
    }

    /// Get a node by ID
    pub fn get(&self, id: &str) -> Option<&Node> {
        self.index.get(id).map(|&idx| &self.inner[idx])
    }

    /// Get a mutable node by ID
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.index.get(id).map(|&idx| &mut self.inner[idx])
    }

    /// Add an edge between two nodes
    pub fn add_edge(&mut self, from: &str, to: &str, edge_type: EdgeType) -> Option<()> {
        let from_idx = *self.index.get(from)?;
        let to_idx = *self.index.get(to)?;

        self.inner.add_edge(from_idx, to_idx, Edge::new(edge_type));
        Some(())
    }

    /// Add a language node that inherits from CO
    pub fn add_language(&mut self, id: &str, name: &str) -> NodeIndex {
        let node = Node::new(
            id.into(),
            TypeKind::Language(LanguageSpec::default()),
            name.into(),
        );
        let idx = self.add_node(node);

        // Language inherits from CO
        if let Some(root_idx) = self.root {
            self.inner
                .add_edge(idx, root_idx, Edge::new(EdgeType::Inherits));
        }

        idx
    }

    /// Add a domain that instantiates a language
    pub fn add_domain(&mut self, id: &str, language_id: &str) -> Option<NodeIndex> {
        let lang_idx = *self.index.get(language_id)?;

        let node = Node::new(id.into(), TypeKind::Domain, id.into());
        let idx = self.add_node(node);

        // Domain inherits from language
        self.inner
            .add_edge(idx, lang_idx, Edge::new(EdgeType::Inherits));

        Some(idx)
    }

    /// Add a definition to a domain
    pub fn define(&mut self, domain_id: &str, symbol: &str, meaning: &str) -> Option<NodeIndex> {
        let domain_idx = *self.index.get(domain_id)?;

        let def_id = format!("{}:{}", domain_id, symbol);
        let node = Node::definition(def_id.clone(), symbol.into(), meaning.into());
        let idx = self.add_node(node);

        // Definition belongs to domain
        self.inner
            .add_edge(idx, domain_idx, Edge::new(EdgeType::BelongsTo));

        Some(idx)
    }

    /// Get all nodes that inherit from a given node
    pub fn inheritors(&self, id: &str) -> Vec<&Node> {
        let Some(&idx) = self.index.get(id) else {
            return vec![];
        };

        self.inner
            .neighbors_directed(idx, petgraph::Direction::Incoming)
            .filter(|&neighbor_idx| {
                self.inner
                    .edges_connecting(neighbor_idx, idx)
                    .any(|e| matches!(e.weight().edge_type, EdgeType::Inherits))
            })
            .map(|neighbor_idx| &self.inner[neighbor_idx])
            .collect()
    }

    /// Get all definitions in a domain
    pub fn definitions(&self, domain_id: &str) -> Vec<&Node> {
        let Some(&idx) = self.index.get(domain_id) else {
            return vec![];
        };

        self.inner
            .neighbors_directed(idx, petgraph::Direction::Incoming)
            .filter(|&neighbor_idx| {
                matches!(self.inner[neighbor_idx].node_type, TypeKind::Definition)
            })
            .map(|neighbor_idx| &self.inner[neighbor_idx])
            .collect()
    }

    /// Count total nodes
    pub fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Count total edges
    pub fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_graph_has_root() {
        let graph = Graph::new();
        assert!(graph.root().is_some());
        assert_eq!(graph.root().unwrap().id, "co");
    }

    #[test]
    fn test_add_language() {
        let mut graph = Graph::new();
        graph.add_language("english", "English");

        assert!(graph.get("english").is_some());
        assert_eq!(graph.inheritors("co").len(), 1);
    }

    #[test]
    fn test_add_domain() {
        let mut graph = Graph::new();
        graph.add_language("english", "English");
        graph.add_domain("my-project", "english");

        assert!(graph.get("my-project").is_some());
        assert_eq!(graph.inheritors("english").len(), 1);
    }

    #[test]
    fn test_define() {
        let mut graph = Graph::new();
        graph.add_language("english", "English");
        graph.add_domain("my-project", "english");
        graph.define("my-project", "sprint", "A fixed time period");

        let defs = graph.definitions("my-project");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].symbol, Some("sprint".into()));
    }
}
