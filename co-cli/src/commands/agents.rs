//! Agents command - list, filter, and show agent definitions
//!
//! Agents are AI-powered extension interfaces defined in `agents/` and `user/agents/`.
//!
//! Examples:
//!   co agents                    # List all agents
//!   co agents type:researcher    # Filter by type
//!   co agents show researcher    # Show agent details
//!   co agents capabilities:search # Filter by capability (in list)

use colored::Colorize;
use comfy_table::{Cell, Table};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Agent item with metadata
#[derive(Debug)]
#[allow(dead_code)]
struct AgentItem {
    id: String,
    agent_type: Option<String>,
    model: Option<String>,
    description: Option<String>,
    path: std::path::PathBuf,
    is_user: bool,
}

/// Frontmatter for agents
#[derive(Debug, Deserialize)]
struct AgentFrontmatter {
    id: Option<String>,
    #[serde(rename = "type")]
    agent_type: Option<String>,
    model: Option<String>,
    capabilities: Option<serde_yaml::Value>,
    #[serde(flatten)]
    fields: HashMap<String, serde_yaml::Value>,
}

/// Run the agents command
pub fn run(action: AgentsAction) {
    match action {
        AgentsAction::List { query } => run_list(&query),
        AgentsAction::Show { name } => run_show(&name),
    }
}

/// Agents subcommand actions
pub enum AgentsAction {
    List { query: Vec<String> },
    Show { name: String },
}

/// List agents with optional filtering
fn run_list(query: &[String]) {
    // Parse field filters
    let filters: Vec<(String, String)> = query
        .iter()
        .filter_map(|arg| {
            let colon_pos = arg.find(':')?;
            let field = &arg[..colon_pos];
            let value = &arg[colon_pos + 1..];
            Some((field.to_string(), value.to_string()))
        })
        .collect();

    // Collect agents from both system and user directories
    let mut agents = Vec::new();

    // System agents
    let agents_dir = Path::new("agents");
    if agents_dir.is_dir() {
        collect_agents(agents_dir, false, &filters, &mut agents);
    }

    // User agents
    let user_agents_dir = Path::new("user/agents");
    if user_agents_dir.is_dir() {
        collect_agents(user_agents_dir, true, &filters, &mut agents);
    }

    if agents.is_empty() {
        if filters.is_empty() {
            println!("{}", "No agents found".yellow());
            println!(
                "{}",
                "Create agents in agents/ or user/agents/ directories".dimmed()
            );
        } else {
            println!("{}", "No agents match the query".yellow());
        }
        return;
    }

    // Display as table
    let mut table = Table::new();
    table.set_header(vec![
        Cell::new("ID").fg(comfy_table::Color::Cyan),
        Cell::new("Type").fg(comfy_table::Color::Cyan),
        Cell::new("Model").fg(comfy_table::Color::Cyan),
        Cell::new("Source").fg(comfy_table::Color::Cyan),
    ]);

    for agent in &agents {
        table.add_row(vec![
            Cell::new(&agent.id).fg(comfy_table::Color::White),
            Cell::new(agent.agent_type.as_deref().unwrap_or("-")),
            Cell::new(agent.model.as_deref().unwrap_or("-")),
            Cell::new(if agent.is_user { "user" } else { "system" }).fg(if agent.is_user {
                comfy_table::Color::Yellow
            } else {
                comfy_table::Color::Green
            }),
        ]);
    }

    println!("{table}");
    println!(
        "\n{} {}",
        agents.len().to_string().cyan(),
        if agents.len() == 1 { "agent" } else { "agents" }
    );
}

/// Collect agents from a directory
fn collect_agents(
    dir: &Path,
    is_user: bool,
    filters: &[(String, String)],
    results: &mut Vec<AgentItem>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "md") {
                if let Some(agent) = parse_agent(&path, is_user, filters) {
                    results.push(agent);
                }
            }
        }
    }
}

/// Parse an agent file and check against filters
fn parse_agent(path: &Path, is_user: bool, filters: &[(String, String)]) -> Option<AgentItem> {
    let content = fs::read_to_string(path).ok()?;

    // Extract frontmatter
    if !content.starts_with("---") {
        return None;
    }

    let end_idx = content[3..].find("\n---")?;
    let yaml_str = &content[4..end_idx + 3];

    let frontmatter: AgentFrontmatter = serde_yaml::from_str(yaml_str).ok()?;

    // Convert all fields to string map for filtering
    let mut string_fields: HashMap<String, String> = HashMap::new();
    for (key, value) in &frontmatter.fields {
        match value {
            serde_yaml::Value::String(s) => {
                string_fields.insert(key.clone(), s.clone());
            }
            serde_yaml::Value::Number(n) => {
                string_fields.insert(key.clone(), n.to_string());
            }
            serde_yaml::Value::Bool(b) => {
                string_fields.insert(key.clone(), b.to_string());
            }
            _ => {}
        }
    }

    // Add agent_type and other known fields
    if let Some(t) = &frontmatter.agent_type {
        string_fields.insert("type".to_string(), t.clone());
    }
    if let Some(m) = &frontmatter.model {
        string_fields.insert("model".to_string(), m.clone());
    }

    // Check filters
    for (field, value) in filters {
        // Handle capabilities (list) specially
        if field == "capabilities" {
            if let Some(caps) = &frontmatter.capabilities {
                if !list_contains(caps, value) {
                    return None;
                }
            } else {
                return None;
            }
        } else {
            // Handle comma-separated values in query (match any)
            let query_values: Vec<&str> = value.split(',').map(|s| s.trim()).collect();
            match string_fields.get(field) {
                Some(v) if query_values.iter().any(|qv| v == *qv) => continue,
                _ => return None,
            }
        }
    }

    // Get ID from frontmatter or filename
    let id = frontmatter.id.clone().unwrap_or_else(|| {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });

    // Extract first line of body as description
    let body_start = end_idx + 7;
    let description = if body_start < content.len() {
        let body = &content[body_start..];
        // Find first non-empty, non-header line
        body.lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| l.trim().to_string())
    } else {
        None
    };

    Some(AgentItem {
        id,
        agent_type: frontmatter.agent_type,
        model: frontmatter.model,
        description,
        path: path.to_path_buf(),
        is_user,
    })
}

/// Check if a YAML value (string or list) contains a value
fn list_contains(value: &serde_yaml::Value, needle: &str) -> bool {
    match value {
        serde_yaml::Value::String(s) => {
            // Comma-separated string
            s.split(',').map(|p| p.trim()).any(|p| p == needle)
        }
        serde_yaml::Value::Sequence(seq) => {
            // YAML list
            seq.iter()
                .any(|v| v.as_str().map(|s| s.trim() == needle).unwrap_or(false))
        }
        _ => false,
    }
}

/// Show details of a specific agent
fn run_show(name: &str) {
    // Search in both directories
    let mut agent_path: Option<std::path::PathBuf> = None;

    // Check system agents first
    let system_path = Path::new("agents").join(format!("{}.md", name));
    if system_path.exists() {
        agent_path = Some(system_path);
    }

    // Check user agents (takes precedence)
    let user_path = Path::new("user/agents").join(format!("{}.md", name));
    if user_path.exists() {
        agent_path = Some(user_path);
    }

    match agent_path {
        Some(path) => {
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "{} Failed to read {}: {}",
                        "error:".red().bold(),
                        path.display(),
                        e
                    );
                    std::process::exit(1);
                }
            };

            // Determine if it's a user agent
            let is_user = path.starts_with("user/");
            let source_label = if is_user {
                "user".yellow()
            } else {
                "system".green()
            };

            println!("{} ({})", name.cyan().bold(), source_label);
            println!("{}", "-".repeat(40));
            println!("{}", content);
        }
        None => {
            eprintln!("{} Agent '{}' not found", "error:".red().bold(), name);

            // List available agents
            let mut available = Vec::new();
            for dir in &["agents", "user/agents"] {
                let path = Path::new(dir);
                if path.is_dir() {
                    if let Ok(entries) = fs::read_dir(path) {
                        for entry in entries.flatten() {
                            let entry_path = entry.path();
                            if entry_path.is_file()
                                && entry_path.extension().is_some_and(|e| e == "md")
                            {
                                if let Some(stem) = entry_path.file_stem() {
                                    available.push(stem.to_string_lossy().to_string());
                                }
                            }
                        }
                    }
                }
            }

            if !available.is_empty() {
                eprintln!("{} {}", "Available agents:".dimmed(), available.join(", "));
            }

            std::process::exit(1);
        }
    }
}
