use crate::context::KnowledgeEntry;
use crate::tool::Tool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Reads Obsidian vaults for context injection.
/// Parses markdown files, follows [[wikilinks]], extracts frontmatter.
pub struct ObsidianTool {
    vault_path: Option<PathBuf>,
}

impl ObsidianTool {
    pub fn new(vault_path: Option<PathBuf>) -> Self {
        Self { vault_path }
    }

    fn vault(&self) -> anyhow::Result<&Path> {
        self.vault_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("No Obsidian vault configured"))
    }
}

impl Tool for ObsidianTool {
    fn name(&self) -> &str {
        "obsidian"
    }

    fn description(&self) -> &str {
        "Obsidian vault reader — search notes, follow wikilinks, extract context"
    }

    fn available(&self) -> bool {
        self.vault_path
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    fn run(&self, action: &str, params: &HashMap<String, String>) -> anyhow::Result<String> {
        match action {
            "search" => self.search(params),
            "read" => self.read_note(params),
            "links" => self.follow_links(params),
            "list_tags" => self.list_tags(params),
            _ => anyhow::bail!("Unknown obsidian action: {}", action),
        }
    }
}

impl ObsidianTool {
    /// Search vault for notes matching a query string
    fn search(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let vault = self.vault()?;
        let query = params
            .get("query")
            .ok_or_else(|| anyhow::anyhow!("Missing 'query' param"))?
            .to_lowercase();

        let mut results: Vec<KnowledgeEntry> = Vec::new();

        for entry in WalkDir::new(vault)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
        {
            let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
            if content.to_lowercase().contains(&query) {
                let rel_path = entry
                    .path()
                    .strip_prefix(vault)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .to_string();

                let tags = extract_tags(&content);
                results.push(KnowledgeEntry {
                    source: rel_path,
                    content: truncate(&content, 2000),
                    tags,
                });
            }

            if results.len() >= 10 {
                break;
            }
        }

        Ok(serde_json::to_string_pretty(&results)?)
    }

    /// Read a specific note by path (relative to vault)
    fn read_note(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let vault = self.vault()?;
        let note = params
            .get("path")
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' param"))?;

        let mut file_path = vault.join(note);
        if !file_path.exists() && file_path.extension().is_none() {
            file_path.set_extension("md");
        }

        if !file_path.exists() {
            anyhow::bail!("Note not found: {}", note);
        }

        Ok(std::fs::read_to_string(&file_path)?)
    }

    /// Follow [[wikilinks]] from a note, returning linked content
    fn follow_links(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let vault = self.vault()?;
        let note = params
            .get("path")
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' param"))?;

        let mut file_path = vault.join(note);
        if !file_path.exists() && file_path.extension().is_none() {
            file_path.set_extension("md");
        }

        let content = std::fs::read_to_string(&file_path)?;
        let links = extract_wikilinks(&content);

        let mut linked: Vec<KnowledgeEntry> = Vec::new();
        for link in links {
            let link_path = vault.join(format!("{}.md", link));
            if let Ok(link_content) = std::fs::read_to_string(&link_path) {
                linked.push(KnowledgeEntry {
                    source: format!("{}.md", link),
                    content: truncate(&link_content, 1000),
                    tags: extract_tags(&link_content),
                });
            }
        }

        Ok(serde_json::to_string_pretty(&linked)?)
    }

    /// List all unique tags across the vault
    fn list_tags(&self, _params: &HashMap<String, String>) -> anyhow::Result<String> {
        let vault = self.vault()?;
        let mut all_tags: Vec<String> = Vec::new();

        for entry in WalkDir::new(vault)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
        {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                for tag in extract_tags(&content) {
                    if !all_tags.contains(&tag) {
                        all_tags.push(tag);
                    }
                }
            }
        }

        all_tags.sort();
        Ok(serde_json::to_string_pretty(&all_tags)?)
    }
}

/// Extract #tags from markdown content
fn extract_tags(content: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for word in content.split_whitespace() {
        if let Some(tag) = word.strip_prefix('#') {
            let tag = tag.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '-');
            if !tag.is_empty()
                && tag
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic())
                    .unwrap_or(false)
            {
                tags.push(format!("#{}", tag));
            }
        }
    }
    tags.sort();
    tags.dedup();
    tags
}

/// Extract [[wikilinks]] from content
fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut remaining = content;

    while let Some(start) = remaining.find("[[") {
        remaining = &remaining[start + 2..];
        if let Some(end) = remaining.find("]]") {
            let link = &remaining[..end];
            // Handle [[link|display]] format
            let link = link.split('|').next().unwrap_or(link).trim();
            if !link.is_empty() {
                links.push(link.to_string());
            }
            remaining = &remaining[end + 2..];
        } else {
            break;
        }
    }

    links
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
