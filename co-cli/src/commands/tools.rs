//! Tools command - list, filter, and show tool definitions
//!
//! Tools are deterministic script/utility extensions defined in `tools/` and `user/tools/`.
//!
//! Examples:
//!   co tools                    # List all tools
//!   co tools category:git       # Filter by category
//!   co tools show gh-wrapper    # Show tool details

use colored::Colorize;
use comfy_table::{Cell, Table};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Tool item with metadata
#[derive(Debug)]
#[allow(dead_code)]
struct ToolItem {
    id: String,
    category: Option<String>,
    tool_type: Option<String>,
    command: Option<String>,
    description: Option<String>,
    path: std::path::PathBuf,
    is_user: bool,
}

/// Frontmatter for tools
#[derive(Debug, Deserialize)]
struct ToolFrontmatter {
    id: Option<String>,
    category: Option<String>,
    tool_type: Option<String>,
    command: Option<String>,
    #[serde(flatten)]
    fields: HashMap<String, serde_yaml::Value>,
}

/// Run the tools command
pub fn run(action: ToolsAction) {
    match action {
        ToolsAction::List { query } => run_list(&query),
        ToolsAction::Show { name } => run_show(&name),
        ToolsAction::Run { name, args } => run_tool(&name, &args),
    }
}

/// Tools subcommand actions
pub enum ToolsAction {
    List { query: Vec<String> },
    Show { name: String },
    Run { name: String, args: Vec<String> },
}

/// List tools with optional filtering
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

    // Collect tools from both system and user directories
    let mut tools = Vec::new();

    // System tools
    let tools_dir = Path::new("tools");
    if tools_dir.is_dir() {
        collect_tools(tools_dir, false, &filters, &mut tools);
    }

    // User tools
    let user_tools_dir = Path::new("user/tools");
    if user_tools_dir.is_dir() {
        collect_tools(user_tools_dir, true, &filters, &mut tools);
    }

    if tools.is_empty() {
        if filters.is_empty() {
            println!("{}", "No tools found".yellow());
            println!(
                "{}",
                "Create tools in tools/ or user/tools/ directories".dimmed()
            );
        } else {
            println!("{}", "No tools match the query".yellow());
        }
        return;
    }

    // Display as table
    let mut table = Table::new();
    table.set_header(vec![
        Cell::new("ID").fg(comfy_table::Color::Cyan),
        Cell::new("Category").fg(comfy_table::Color::Cyan),
        Cell::new("Command").fg(comfy_table::Color::Cyan),
        Cell::new("Source").fg(comfy_table::Color::Cyan),
    ]);

    for tool in &tools {
        table.add_row(vec![
            Cell::new(&tool.id).fg(comfy_table::Color::White),
            Cell::new(tool.category.as_deref().unwrap_or("-")),
            Cell::new(tool.command.as_deref().unwrap_or("-")),
            Cell::new(if tool.is_user { "user" } else { "system" }).fg(if tool.is_user {
                comfy_table::Color::Yellow
            } else {
                comfy_table::Color::Green
            }),
        ]);
    }

    println!("{table}");
    println!(
        "\n{} {}",
        tools.len().to_string().cyan(),
        if tools.len() == 1 { "tool" } else { "tools" }
    );
}

/// Collect tools from a directory
fn collect_tools(
    dir: &Path,
    is_user: bool,
    filters: &[(String, String)],
    results: &mut Vec<ToolItem>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|e| e == "md")
                && let Some(tool) = parse_tool(&path, is_user, filters)
            {
                results.push(tool);
            }
        }
    }
}

/// Parse a tool file and check against filters
fn parse_tool(path: &Path, is_user: bool, filters: &[(String, String)]) -> Option<ToolItem> {
    let content = fs::read_to_string(path).ok()?;

    // Extract frontmatter
    if !content.starts_with("---") {
        return None;
    }

    let end_idx = content[3..].find("\n---")?;
    let yaml_str = &content[4..end_idx + 3];

    let frontmatter: ToolFrontmatter = serde_yaml::from_str(yaml_str).ok()?;

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

    // Add category and other known fields
    if let Some(c) = &frontmatter.category {
        string_fields.insert("category".to_string(), c.clone());
    }
    if let Some(cmd) = &frontmatter.command {
        string_fields.insert("command".to_string(), cmd.clone());
    }

    // Check filters
    for (field, value) in filters {
        // Handle comma-separated values in query (match any)
        let query_values: Vec<&str> = value.split(',').map(|s| s.trim()).collect();
        match string_fields.get(field) {
            Some(v) if query_values.iter().any(|qv| v == *qv) => continue,
            _ => return None,
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

    Some(ToolItem {
        id,
        category: frontmatter.category,
        tool_type: frontmatter.tool_type,
        command: frontmatter.command,
        description,
        path: path.to_path_buf(),
        is_user,
    })
}

/// Show details of a specific tool
fn run_show(name: &str) {
    // Search in both directories
    let mut tool_path: Option<std::path::PathBuf> = None;

    // Check system tools first
    let system_path = Path::new("tools").join(format!("{}.md", name));
    if system_path.exists() {
        tool_path = Some(system_path);
    }

    // Check user tools (takes precedence)
    let user_path = Path::new("user/tools").join(format!("{}.md", name));
    if user_path.exists() {
        tool_path = Some(user_path);
    }

    match tool_path {
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

            // Determine if it's a user tool
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
            eprintln!("{} Tool '{}' not found", "error:".red().bold(), name);

            // List available tools
            let mut available = Vec::new();
            for dir in &["tools", "user/tools"] {
                let path = Path::new(dir);
                if path.is_dir()
                    && let Ok(entries) = fs::read_dir(path)
                {
                    for entry in entries.flatten() {
                        let entry_path = entry.path();
                        if entry_path.is_file()
                            && entry_path.extension().is_some_and(|e| e == "md")
                            && let Some(stem) = entry_path.file_stem()
                        {
                            available.push(stem.to_string_lossy().to_string());
                        }
                    }
                }
            }

            if !available.is_empty() {
                eprintln!("{} {}", "Available tools:".dimmed(), available.join(", "));
            }

            std::process::exit(1);
        }
    }
}

/// Run a tool by name with arguments
fn run_tool(name: &str, args: &[String]) {
    // Load tool definition (user tools take precedence)
    let tool = match load_tool(name) {
        Some(t) => t,
        None => {
            eprintln!("{} Tool '{}' not found", "error:".red().bold(), name);
            list_available_tools();
            std::process::exit(1);
        }
    };

    // Check tool_type (default to deterministic)
    let tool_type = tool.tool_type.as_deref().unwrap_or("deterministic");

    match tool_type {
        "predictive" => {
            eprintln!(
                "{} Predictive tools not yet implemented",
                "error:".red().bold()
            );
            eprintln!(
                "{}",
                "Future support: whisper, pdf-summarizer, video-transcriber".dimmed()
            );
            std::process::exit(1);
        }
        _ => {
            // Default to deterministic behavior
            // Require command field for deterministic tools
            let command = match &tool.command {
                Some(cmd) => cmd,
                None => {
                    eprintln!(
                        "{} Tool '{}' has no executable command",
                        "error:".red().bold(),
                        name
                    );
                    eprintln!(
                        "{}",
                        "Add a 'command' field to the tool frontmatter".dimmed()
                    );
                    std::process::exit(1);
                }
            };

            execute_command(command, args);
        }
    }
}

/// Load a tool by name from user/tools or tools directories
fn load_tool(name: &str) -> Option<ToolItem> {
    // Check user tools first (takes precedence)
    let user_path = Path::new("user/tools").join(format!("{}.md", name));
    if user_path.exists() {
        return parse_tool(&user_path, true, &[]);
    }

    // Check system tools
    let system_path = Path::new("tools").join(format!("{}.md", name));
    if system_path.exists() {
        return parse_tool(&system_path, false, &[]);
    }

    None
}

/// List available tools for error messages
fn list_available_tools() {
    let mut available = Vec::new();

    for dir in &["tools", "user/tools"] {
        let path = Path::new(dir);
        if path.is_dir()
            && let Ok(entries) = fs::read_dir(path)
        {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_file()
                    && entry_path.extension().is_some_and(|e| e == "md")
                    && let Some(stem) = entry_path.file_stem()
                {
                    available.push(stem.to_string_lossy().to_string());
                }
            }
        }
    }

    if !available.is_empty() {
        eprintln!("{} {}", "Available tools:".dimmed(), available.join(", "));
    }
}

/// Execute a shell command with arguments
fn execute_command(command: &str, args: &[String]) {
    use std::process::Command;

    // Split the command into program and initial args
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        eprintln!("{} Empty command", "error:".red().bold());
        std::process::exit(1);
    }

    let program = parts[0];
    let mut cmd = Command::new(program);

    // Add command's built-in args
    for arg in &parts[1..] {
        cmd.arg(arg);
    }

    // Add user-provided args
    for arg in args {
        cmd.arg(arg);
    }

    // Execute and handle result
    match cmd.status() {
        Ok(status) => {
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        Err(e) => {
            eprintln!(
                "{} Failed to execute '{}': {}",
                "error:".red().bold(),
                program,
                e
            );
            std::process::exit(1);
        }
    }
}
