//! Embedded documentation for co help system
//!
//! This module provides topic-based help accessible via `co help <topic>`.

/// Getting started guide
pub const GETTING_STARTED: &str = include_str!("getting_started.md");

/// Understanding spaces
pub const SPACES: &str = include_str!("concepts/spaces.md");

/// Understanding workflows
pub const WORKFLOWS: &str = include_str!("concepts/workflows.md");

/// Understanding work items
pub const WORK_ITEMS: &str = include_str!("concepts/work_items.md");

/// All available topics with descriptions
pub const TOPICS: &[(&str, &str)] = &[
    ("getting-started", "Quick start guide for CO"),
    ("spaces", "Understanding spaces and namespaces"),
    ("workflows", "Plan & Execute, Write workflows"),
    ("work-items", "User-stories, tasks, epics, releases"),
];

/// Get documentation content for a topic
pub fn get_topic(name: &str) -> Option<&'static str> {
    match name {
        "getting-started" | "start" | "intro" => Some(GETTING_STARTED),
        "spaces" | "space" | "namespaces" => Some(SPACES),
        "workflows" | "workflow" | "conduct" | "write" => Some(WORKFLOWS),
        "work-items" | "work-item" | "items" | "types" => Some(WORK_ITEMS),
        _ => None,
    }
}

/// List all available topics
pub fn list_topics() -> &'static [(&'static str, &'static str)] {
    TOPICS
}

/// Find similar topic names for suggestions
pub fn find_similar(name: &str) -> Vec<&'static str> {
    let name_lower = name.to_lowercase();
    TOPICS
        .iter()
        .filter(|(topic, desc)| {
            topic.contains(&name_lower) || desc.to_lowercase().contains(&name_lower)
        })
        .map(|(topic, _)| *topic)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_topic_exact_match() {
        assert!(get_topic("getting-started").is_some());
        assert!(get_topic("spaces").is_some());
    }

    #[test]
    fn test_get_topic_aliases() {
        assert!(get_topic("start").is_some());
        assert!(get_topic("intro").is_some());
        assert!(get_topic("space").is_some());
    }

    #[test]
    fn test_get_topic_unknown() {
        assert!(get_topic("nonexistent").is_none());
    }

    #[test]
    fn test_list_topics_not_empty() {
        assert!(!list_topics().is_empty());
    }

    #[test]
    fn test_find_similar() {
        let similar = find_similar("space");
        assert!(similar.contains(&"spaces"));
    }
}
