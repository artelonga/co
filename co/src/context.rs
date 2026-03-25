use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Context flows through the pipeline, accumulating artifacts at each stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    /// Unique run ID
    pub run_id: String,
    /// Target repository (owner/repo)
    pub repo: String,
    /// Working directory for the repo (if cloned/local)
    pub workdir: Option<String>,
    /// High-level objective
    pub title: String,
    /// Detailed description / user intent
    pub description: String,
    /// Knowledge entries gathered from vault or other sources
    pub knowledge: Vec<KnowledgeEntry>,
    /// Artifacts produced by pipeline stages
    pub artifacts: HashMap<String, String>,
    /// Current pipeline stage
    pub stage: Stage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub source: String,
    pub content: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Plan,
    Execute,
    Approve,
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stage::Plan => write!(f, "plan"),
            Stage::Execute => write!(f, "execute"),
            Stage::Approve => write!(f, "approve"),
        }
    }
}

impl Context {
    pub fn new(repo: &str, title: &str, description: &str) -> Self {
        Self {
            run_id: uuid::Uuid::new_v4().to_string(),
            repo: repo.into(),
            workdir: None,
            title: title.into(),
            description: description.into(),
            knowledge: Vec::new(),
            artifacts: HashMap::new(),
            stage: Stage::Plan,
        }
    }

    /// Format context for LLM prompt injection
    pub fn to_prompt_context(&self) -> String {
        let mut parts = vec![
            format!("# Context: {}", self.title),
            format!("Repository: {}", self.repo),
            format!("Stage: {}", self.stage),
            String::new(),
            self.description.clone(),
        ];

        if !self.knowledge.is_empty() {
            parts.push(String::new());
            parts.push("## Knowledge".into());
            for entry in &self.knowledge {
                parts.push(format!("### {} [{}]", entry.source, entry.tags.join(", ")));
                parts.push(entry.content.clone());
            }
        }

        if !self.artifacts.is_empty() {
            parts.push(String::new());
            parts.push("## Artifacts".into());
            for (key, value) in &self.artifacts {
                parts.push(format!("### {}", key));
                parts.push(value.clone());
            }
        }

        parts.join("\n")
    }
}
