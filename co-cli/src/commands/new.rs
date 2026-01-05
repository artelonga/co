//! Create new content files
//!
//! Creates content files (tasks, definitions, etc.) in a specified space.

use co::validate::ValidationContext;
use colored::Colorize;
use std::fs;
use std::path::Path;

/// Run the new command
pub fn run(content_type: &str, name: &str, space: Option<&str>) {
    // Use current directory if no --in flag provided
    let space_dir = space.unwrap_or(".");
    let space_path = Path::new(space_dir);

    // Verify space exists (current dir always exists, but explicit ones might not)
    if !space_path.exists() {
        eprintln!(
            "{} Directory '{}' does not exist",
            "error:".red().bold(),
            space_dir
        );
        std::process::exit(1);
    }

    // Derive a sensible space name for frontmatter
    let space_name = if space_dir == "." {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "root".to_string())
    } else {
        space_dir.to_string()
    };

    // Verify content type is known (built-in or registered via schema)
    let ctx = ValidationContext::new(Path::new("."));
    if !ctx.type_exists(content_type) {
        eprintln!("{} Unknown type: '{}'", "error:".red().bold(), content_type);
        eprintln!(
            "{}",
            "Create a schema.yaml with content_types to register custom types.".dimmed()
        );
        std::process::exit(1);
    }

    // Create type directory (e.g., tasks/, definitions/)
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

    // Create the content file
    let file_path = type_dir.join(format!("{}.md", name));

    let content = format!(
        r#"---
schema_version: 2
language: en
space: {}
type: {}
id: {}
status: todo
---

# {}
"#,
        space_name, content_type, name, name
    );

    if let Err(e) = fs::write(&file_path, content) {
        eprintln!(
            "{} Failed to create {}: {}",
            "error:".red().bold(),
            file_path.display(),
            e
        );
        std::process::exit(1);
    }

    println!("{}", "Created".bold().green());
    println!("{}", "─".repeat(30));
    println!("  {}: {}", "file".cyan(), file_path.display());
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
