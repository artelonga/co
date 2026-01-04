//! Write command - generate content using writer agents
//!
//! Invokes a writer agent to create content based on its backend:
//! - manual: Interactive prompts (like co create user role)
//! - claude: Skeleton template for Claude Code to fill
//! - ollama: Local model integration (stub for future)
//!
//! Examples:
//!   co write user-story --agent writer --in private
//!   co write task --agent writer --context notes.md

use co::validate::ValidationContext;
use colored::Colorize;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

/// Agent configuration loaded from frontmatter
#[derive(Debug)]
struct AgentConfig {
    id: String,
    backend: String,
    model: Option<String>,
    capabilities: Option<String>,
    #[allow(dead_code)]
    context: Option<String>,
}

/// Frontmatter for agents
#[derive(Debug, Deserialize)]
struct AgentFrontmatter {
    id: Option<String>,
    backend: Option<String>,
    model: Option<String>,
    capabilities: Option<serde_yaml::Value>,
    context: Option<serde_yaml::Value>,
    #[serde(flatten)]
    #[allow(dead_code)]
    fields: HashMap<String, serde_yaml::Value>,
}

/// Run the write command
pub fn run(
    content_type: &str,
    agent_name: &str,
    space: Option<&str>,
    context_file: Option<&str>,
    name: Option<&str>,
) {
    // 1. Load and validate agent
    let agent = match load_agent(agent_name) {
        Some(a) => a,
        None => {
            eprintln!("{} Agent '{}' not found", "error:".red().bold(), agent_name);
            list_available_agents();
            std::process::exit(1);
        }
    };

    // 2. Determine target space
    let space_name = space.unwrap_or("work");
    let space_path = Path::new(space_name);

    // Verify space exists
    if !space_path.exists() {
        eprintln!(
            "{} Space '{}' does not exist",
            "error:".red().bold(),
            space_name
        );
        std::process::exit(1);
    }

    // 3. Validate content type
    let ctx = ValidationContext::new(Path::new("."));
    if !ctx.type_exists(content_type) {
        eprintln!("{} Unknown type: '{}'", "error:".red().bold(), content_type);
        eprintln!(
            "{}",
            "Create a schema.yaml with content_types to register custom types.".dimmed()
        );
        std::process::exit(1);
    }

    // 4. Load context file if provided
    let context_content = context_file.and_then(|path| {
        fs::read_to_string(path)
            .map_err(|e| {
                eprintln!(
                    "{} Failed to read context file '{}': {}",
                    "error:".red().bold(),
                    path,
                    e
                );
            })
            .ok()
    });

    // 5. Dispatch based on backend
    match agent.backend.as_str() {
        "manual" => run_manual(content_type, space_name, name, context_content.as_deref()),
        "claude" => run_claude(
            content_type,
            space_name,
            name,
            &agent,
            context_content.as_deref(),
        ),
        "ollama" => {
            eprintln!(
                "{} ollama backend not yet implemented",
                "error:".red().bold()
            );
            std::process::exit(1);
        }
        other => {
            eprintln!(
                "{} Unknown backend '{}'. Use: manual, claude, ollama",
                "error:".red().bold(),
                other
            );
            std::process::exit(1);
        }
    }
}

/// Load agent configuration from agents/ or user/agents/
fn load_agent(name: &str) -> Option<AgentConfig> {
    // Check user agents first (takes precedence)
    let user_path = Path::new("user/agents").join(format!("{}.md", name));
    if user_path.exists() {
        return parse_agent_config(&user_path, name);
    }

    // Check system agents
    let system_path = Path::new("agents").join(format!("{}.md", name));
    if system_path.exists() {
        return parse_agent_config(&system_path, name);
    }

    None
}

/// Parse agent frontmatter into config
fn parse_agent_config(path: &Path, name: &str) -> Option<AgentConfig> {
    let content = fs::read_to_string(path).ok()?;

    // Extract frontmatter
    if !content.starts_with("---") {
        return None;
    }

    let end_idx = content[3..].find("\n---")?;
    let yaml_str = &content[4..end_idx + 3];

    let frontmatter: AgentFrontmatter = serde_yaml::from_str(yaml_str).ok()?;

    let capabilities = frontmatter.capabilities.and_then(|v| match v {
        serde_yaml::Value::String(s) => Some(s),
        serde_yaml::Value::Sequence(seq) => {
            let caps: Vec<String> = seq
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            Some(caps.join(", "))
        }
        _ => None,
    });

    let context = frontmatter.context.and_then(|v| match v {
        serde_yaml::Value::String(s) => Some(s),
        serde_yaml::Value::Sequence(seq) => {
            let items: Vec<String> = seq
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            Some(items.join(", "))
        }
        _ => None,
    });

    Some(AgentConfig {
        id: frontmatter.id.unwrap_or_else(|| name.to_string()),
        backend: frontmatter.backend.unwrap_or_else(|| "manual".to_string()),
        model: frontmatter.model,
        capabilities,
        context,
    })
}

/// List available agents
fn list_available_agents() {
    let mut available = Vec::new();

    for dir in &["agents", "user/agents"] {
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
        eprintln!("{} {}", "Available agents:".dimmed(), available.join(", "));
    }
}

/// Run manual backend - interactive prompts
fn run_manual(content_type: &str, space: &str, provided_name: Option<&str>, context: Option<&str>) {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    // Get name
    let name = if let Some(n) = provided_name {
        n.to_string()
    } else {
        print!("{}", "Name: ".cyan());
        let _ = stdout.flush();
        read_line()
    };

    // Get context (multi-line, ends with empty line)
    let context_content = if let Some(ctx) = context {
        ctx.to_string()
    } else {
        println!("{}", "Enter context (press Enter twice when done):".cyan());
        let _ = stdout.flush();

        let mut context_lines = Vec::new();
        loop {
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(_) => {
                    let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                    if trimmed.is_empty() {
                        break;
                    }
                    context_lines.push(trimmed.to_string());
                }
                Err(_) => break,
            }
        }
        context_lines.join("\n")
    };

    // Generate content based on type
    let content = match content_type {
        "user-story" => {
            print!("{}", "AS A: ".cyan().bold());
            let _ = stdout.flush();
            let as_a = read_line();

            print!("{}", "I NEED: ".cyan().bold());
            let _ = stdout.flush();
            let i_need = read_line();

            print!("{}", "SO THAT: ".cyan().bold());
            let _ = stdout.flush();
            let so_that = read_line();

            format!(
                r#"---
schema_version: 2
language: en
space: {}
type: user-story
id: {}
status: todo
---

# {}

## Prompt
{}

## As
{}

## I Need
{}

## To
{}
"#,
                space, name, name, context_content, as_a, i_need, so_that
            )
        }
        "task" => {
            print!("{}", "GIVEN: ".cyan().bold());
            let _ = stdout.flush();
            let given = read_line();

            print!("{}", "WHEN: ".cyan().bold());
            let _ = stdout.flush();
            let when = read_line();

            print!("{}", "THEN: ".cyan().bold());
            let _ = stdout.flush();
            let then = read_line();

            format!(
                r#"---
schema_version: 2
language: en
space: {}
type: task
id: {}
status: todo
---

# {}

## Prompt
{}

## Given
{}

## When
{}

## Then
{}
"#,
                space, name, name, context_content, given, when, then
            )
        }
        _ => {
            format!(
                r#"---
schema_version: 2
language: en
space: {}
type: {}
id: {}
status: todo
---

# {}

## Prompt
{}
"#,
                space, content_type, name, name, context_content
            )
        }
    };

    // Write file
    write_content_file(content_type, space, &name, &content);

    println!("{}", "Created".bold().green());
    println!("{}", "─".repeat(30));
    let file_path = Path::new(space)
        .join(pluralize(content_type))
        .join(format!("{}.md", name));
    println!("  {}: {}", "file".cyan(), file_path.display());
}

/// Run claude backend - create skeleton for Claude Code
fn run_claude(
    content_type: &str,
    space: &str,
    provided_name: Option<&str>,
    agent: &AgentConfig,
    context: Option<&str>,
) {
    let mut stdout = io::stdout();

    // Get name
    let name = if let Some(n) = provided_name {
        n.to_string()
    } else {
        print!("{}", "Name: ".cyan());
        let _ = stdout.flush();
        read_line()
    };

    // Build prompt section with agent info and context
    let mut prompt_parts = Vec::new();

    prompt_parts.push(format!("Agent: {} ({})", agent.id, agent.backend));
    if let Some(model) = &agent.model {
        prompt_parts.push(format!("Model: {}", model));
    }
    if let Some(caps) = &agent.capabilities {
        prompt_parts.push(format!("Capabilities: {}", caps));
    }
    prompt_parts.push(String::new());
    prompt_parts.push("Instructions: Fill in the sections below based on the context.".to_string());

    if let Some(ctx) = context {
        prompt_parts.push(String::new());
        prompt_parts.push("Context:".to_string());
        prompt_parts.push(ctx.to_string());
    }

    let prompt_content = prompt_parts.join("\n");

    // Generate skeleton based on type
    let content = match content_type {
        "user-story" => format!(
            r#"---
schema_version: 2
language: en
space: {}
type: user-story
id: {}
status: todo
---

# {}

## Prompt
{}

## As


## I Need


## To

"#,
            space, name, name, prompt_content
        ),
        "task" => format!(
            r#"---
schema_version: 2
language: en
space: {}
type: task
id: {}
status: todo
---

# {}

## Prompt
{}

## Given


## When


## Then

"#,
            space, name, name, prompt_content
        ),
        _ => format!(
            r#"---
schema_version: 2
language: en
space: {}
type: {}
id: {}
status: todo
---

# {}

## Prompt
{}
"#,
            space, content_type, name, name, prompt_content
        ),
    };

    // Write file
    write_content_file(content_type, space, &name, &content);

    println!("{}", "Created skeleton".bold().green());
    println!("{}", "─".repeat(30));
    let file_path = Path::new(space)
        .join(pluralize(content_type))
        .join(format!("{}.md", name));
    println!("  {}: {}", "file".cyan(), file_path.display());
}

/// Write content to file
fn write_content_file(content_type: &str, space: &str, name: &str, content: &str) {
    let type_dir = Path::new(space).join(pluralize(content_type));

    if let Err(e) = fs::create_dir_all(&type_dir) {
        eprintln!(
            "{} Failed to create {}: {}",
            "error:".red().bold(),
            type_dir.display(),
            e
        );
        std::process::exit(1);
    }

    let file_path = type_dir.join(format!("{}.md", name));
    if let Err(e) = fs::write(&file_path, content) {
        eprintln!(
            "{} Failed to create {}: {}",
            "error:".red().bold(),
            file_path.display(),
            e
        );
        std::process::exit(1);
    }
}

/// Read a single line from stdin
fn read_line() -> String {
    let stdin = io::stdin();
    let mut line = String::new();
    match stdin.lock().read_line(&mut line) {
        Ok(_) => line.trim().to_string(),
        Err(_) => String::new(),
    }
}

/// Pluralize a content type name for directory naming
fn pluralize(word: &str) -> String {
    if word.ends_with("-story") {
        return word.replace("-story", "-stories");
    }
    if word.ends_with("y")
        && !word.ends_with("ey")
        && !word.ends_with("ay")
        && !word.ends_with("oy")
    {
        return format!("{}ies", &word[..word.len() - 1]);
    }
    if word.ends_with("s") || word.ends_with("x") || word.ends_with("ch") || word.ends_with("sh") {
        return format!("{}es", word);
    }
    format!("{}s", word)
}
