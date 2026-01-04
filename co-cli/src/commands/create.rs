//! Create command - interactive content creation with role selection
//!
//! Enables collaborative content creation between user and agent (Claude Code).
//! - User role: Interactive prompts for structured input
//! - Agent role: Creates skeleton template for Claude Code to fill in

use crate::i18n::load_i18n;
use co::validate::ValidationContext;
use colored::Colorize;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

/// Run the create command
pub fn run(content_type: &str, name: &str, space: Option<&str>, story: Option<&str>) {
    let i18n = load_i18n();

    // Determine target space
    let space_name = space.unwrap_or("en");
    let space_path = Path::new(space_name);

    // Verify space exists
    if !space_path.exists() {
        eprintln!(
            "{} {} '{}' does not exist",
            "error:".red().bold(),
            i18n.type_label("space"),
            space_name
        );
        std::process::exit(1);
    }

    // Verify content type is known
    let ctx = ValidationContext::new(Path::new("."));
    if !ctx.type_exists(content_type) {
        eprintln!("{} Unknown type: '{}'", "error:".red().bold(), content_type);
        eprintln!(
            "{}",
            "Create a schema.yaml with content_types to register custom types.".dimmed()
        );
        std::process::exit(1);
    }

    // Prompt for role selection
    let role = prompt_role();

    // Create content based on role and type
    let content = match role.as_str() {
        "agent" => create_skeleton(content_type, name, space_name, story),
        "user" => create_with_prompts(content_type, name, space_name, story),
        _ => {
            eprintln!(
                "{} Invalid role: '{}'. Use 'user' or 'agent'.",
                "error:".red().bold(),
                role
            );
            std::process::exit(1);
        }
    };

    // Create type directory
    let type_dir = space_path.join(pluralize(content_type));
    if let Err(e) = fs::create_dir_all(&type_dir) {
        eprintln!(
            "{} Failed to create {}: {}",
            "error:".red().bold(),
            type_dir.display(),
            e
        );
        std::process::exit(1);
    }

    // Write the file
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

    if role == "agent" {
        println!("{}", "Created skeleton".bold().green());
    } else {
        println!("{}", "Created".bold().green());
    }
    println!("{}", "─".repeat(30));
    println!("  {}: {}", "file".cyan(), file_path.display());
}

/// Prompt for role selection (user/agent)
fn prompt_role() -> String {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    print!("{}", "Who is creating? [user/agent]: ".cyan());
    let _ = stdout.flush();

    let mut line = String::new();
    match stdin.lock().read_line(&mut line) {
        Ok(_) => {
            let role = line.trim().to_lowercase();
            if role.is_empty() {
                "user".to_string() // Default to user
            } else {
                role
            }
        }
        Err(_) => "user".to_string(),
    }
}

/// Create skeleton content for agent role
fn create_skeleton(content_type: &str, name: &str, space: &str, story: Option<&str>) -> String {
    let story_line = story
        .map(|s| format!("story: [[{}]]\n", s))
        .unwrap_or_default();

    match content_type {
        "user-story" => format!(
            r#"---
schema_version: 2
language: en
space: {}
type: user-story
id: {}
status: todo
{}---

# {}

## Prompt


## As


## I Need


## To

"#,
            space, name, story_line, name
        ),
        "task" => format!(
            r#"---
schema_version: 2
language: en
space: {}
type: task
id: {}
status: todo
{}---

# {}

## Prompt


## Given


## When


## Then

"#,
            space, name, story_line, name
        ),
        _ => format!(
            r#"---
schema_version: 2
language: en
space: {}
type: {}
id: {}
status: todo
{}---

# {}

## Prompt

"#,
            space, content_type, name, story_line, name
        ),
    }
}

/// Create content with interactive prompts for user role
fn create_with_prompts(content_type: &str, name: &str, space: &str, story: Option<&str>) -> String {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    // Prompt for context (multi-line, ends with empty line)
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
    let context = context_lines.join("\n");

    let story_line = story
        .map(|s| format!("story: [[{}]]\n", s))
        .unwrap_or_default();

    match content_type {
        "user-story" => {
            // Prompt for AS A / I NEED / SO THAT
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
{}---

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
                space, name, story_line, name, context, as_a, i_need, so_that
            )
        }
        "task" => {
            // Prompt for GIVEN / WHEN / THEN
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
{}---

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
                space, name, story_line, name, context, given, when, then
            )
        }
        _ => {
            // Generic content type - just use context
            format!(
                r#"---
schema_version: 2
language: en
space: {}
type: {}
id: {}
status: todo
{}---

# {}

## Prompt
{}
"#,
                space, content_type, name, story_line, name, context
            )
        }
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
    // Handle special cases
    if word.ends_with("-story") {
        return word.replace("-story", "-stories");
    }
    if word.ends_with("y")
        && !word.ends_with("ey")
        && !word.ends_with("ay")
        && !word.ends_with("oy")
    {
        // Words ending in consonant + y: change y to ies
        return format!("{}ies", &word[..word.len() - 1]);
    }
    if word.ends_with("s") || word.ends_with("x") || word.ends_with("ch") || word.ends_with("sh") {
        return format!("{}es", word);
    }
    // Default: just add s
    format!("{}s", word)
}
