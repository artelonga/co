//! Create new content files
//!
//! Creates content files (tasks, definitions, etc.) in a specified context.

use crate::i18n::load_i18n;
use colored::Colorize;
use std::fs;
use std::path::Path;

/// Run the new command
pub fn run(content_type: &str, name: &str, scope: Option<&str>) {
    let i18n = load_i18n();

    // Determine target scope
    let scope_name = scope.unwrap_or("en");
    let scope_path = Path::new(scope_name);

    // Verify scope exists
    if !scope_path.exists() {
        eprintln!(
            "{} {} '{}' does not exist",
            "error:".red().bold(),
            i18n.type_label("context"),
            scope_name
        );
        std::process::exit(1);
    }

    // Create type directory (e.g., tasks/, definitions/)
    let type_dir = scope_path.join(format!("{}s", content_type));
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
scope: {}
type: {}
id: {}
status: todo
---

# {}
"#,
        scope_name, content_type, name, name
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
