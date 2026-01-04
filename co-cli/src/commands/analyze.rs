//! Analyze content quality and generate suggestions
//!
//! Evaluates content items against type-specific criteria, identifies gaps,
//! and generates actionable suggestions and interview questions.

use co::ValidationContext;
use co::content::{ParsedContent, specs_for_type};
use colored::Colorize;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Analysis check result
struct Check {
    passed: bool,
    warning: bool,
    message: String,
}

impl Check {
    fn pass(message: &str) -> Self {
        Self {
            passed: true,
            warning: false,
            message: message.to_string(),
        }
    }

    fn warning(message: &str) -> Self {
        Self {
            passed: true,
            warning: true,
            message: message.to_string(),
        }
    }

    fn fail(message: &str) -> Self {
        Self {
            passed: false,
            warning: false,
            message: message.to_string(),
        }
    }
}

/// Analysis result for a content item
struct AnalysisResult {
    item_type: String,
    title: Option<String>,
    status: Option<String>,
    checks: Vec<Check>,
    suggestions: Vec<String>,
    questions: Vec<String>,
}

/// Frontmatter structure for analysis
#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct Frontmatter {
    #[serde(rename = "type")]
    content_type: Option<String>,
    id: Option<String>,
    status: Option<String>,
    language: Option<String>,
    story: Option<String>,
    epic: Option<String>,
    assignee: Option<String>,
    priority: Option<String>,
}

/// Run content analysis on a specific item
pub fn run(name: &str, verbose: bool) {
    let current_dir = Path::new(".");

    // Find the file
    let file_path = match find_file(current_dir, name) {
        Some(path) => path,
        None => {
            eprintln!("{} Item '{}' not found", "error:".red().bold(), name);
            std::process::exit(1);
        }
    };

    // Read file content
    let content = match fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} Failed to read file: {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    };

    // Build validation context for link checking
    let mut ctx = ValidationContext::new(current_dir);
    collect_known_ids(current_dir, &mut ctx.known_ids);

    // Analyze the content
    let result = analyze_content(&content, &ctx);

    // Print results
    print_result(&result, verbose);
}

/// Analyze content and generate result
fn analyze_content(content: &str, ctx: &ValidationContext) -> AnalysisResult {
    let mut checks = Vec::new();
    let mut suggestions = Vec::new();
    let mut questions = Vec::new();

    // Parse frontmatter
    let (frontmatter, body) = parse_frontmatter(content);
    let content_type = frontmatter.content_type.clone().unwrap_or_default();

    // Parse body for title and sections
    let specs = specs_for_type(&content_type).unwrap_or_default();
    let parsed = ParsedContent::parse(&body, &specs);

    // Check 1: Has clear title
    if parsed.title.is_some() {
        checks.push(Check::pass("Has clear title"));
    } else {
        checks.push(Check::fail("Missing title (# heading)"));
        suggestions.push("Add a clear title using # heading".to_string());
    }

    // Check 2: Has status field
    if frontmatter.status.is_some() {
        checks.push(Check::pass("Has status field"));
    } else {
        checks.push(Check::warning("Missing status field"));
        suggestions.push("Set status field (draft, in-progress, done)".to_string());
    }

    // Check 3: Required sections (type-specific)
    let missing_sections = parsed.validate(&specs);
    if missing_sections.is_empty() && !specs.is_empty() {
        checks.push(Check::pass("Has all required sections"));
    } else {
        for msg in &missing_sections {
            checks.push(Check::fail(msg));
        }

        // Generate questions for missing sections
        for spec in &specs {
            if !parsed.sections.contains_key(&spec.field) && spec.required {
                let question = match spec.header.to_lowercase().as_str() {
                    "as" => "Who is the primary user for this story?".to_string(),
                    "i need" => "What specific functionality is needed?".to_string(),
                    "to" => "What value does this deliver?".to_string(),
                    "given" => "What is the initial context or state?".to_string(),
                    "when" => "What action triggers the behavior?".to_string(),
                    "then" => "What is the expected outcome?".to_string(),
                    _ => format!("What should go in the {} section?", spec.header),
                };
                questions.push(question);
            }
        }
    }

    // Check 4: Broken links
    let internal_links = extract_internal_links(&body);
    for link in &internal_links {
        if !ctx.known_ids.contains(link) {
            checks.push(Check::warning(&format!("Broken link: {}", link)));
            suggestions.push(format!("Fix or remove broken link: [[{}]]", link));
        }
    }

    // Check 5: Parent link for tasks
    if content_type == "task" && frontmatter.story.is_none() {
        checks.push(Check::warning("No parent story linked"));
        suggestions.push("Link to parent story using story: field".to_string());
        questions.push("Which user story does this task belong to?".to_string());
    }

    // Check 6: Assignee
    if frontmatter.assignee.is_none() && frontmatter.status.as_deref() == Some("in-progress") {
        suggestions.push("Consider adding an assignee".to_string());
        questions.push("Who should be assigned to this?".to_string());
    }

    // Check 7: Priority
    if frontmatter.priority.is_none() {
        questions.push("What is the priority level?".to_string());
    }

    AnalysisResult {
        item_type: format_type_name(&content_type),
        title: parsed.title,
        status: frontmatter.status,
        checks,
        suggestions,
        questions,
    }
}

/// Parse YAML frontmatter from content
fn parse_frontmatter(content: &str) -> (Frontmatter, String) {
    if !content.starts_with("---") {
        return (Frontmatter::default(), content.to_string());
    }

    let rest = &content[3..];
    if let Some(end_idx) = rest.find("\n---") {
        let yaml_str = &rest[1..end_idx];
        let body = &rest[end_idx + 4..];

        let frontmatter: Frontmatter = serde_yaml::from_str(yaml_str).unwrap_or_default();
        return (frontmatter, body.to_string());
    }

    (Frontmatter::default(), content.to_string())
}

/// Extract internal [[links]] from content
fn extract_internal_links(body: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut chars = body.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '[' && chars.peek() == Some(&'[') {
            chars.next(); // consume second '['
            let mut link = String::new();
            while let Some(c) = chars.next() {
                if c == ']' && chars.peek() == Some(&']') {
                    chars.next();
                    if !link.is_empty() {
                        links.push(link);
                    }
                    break;
                }
                link.push(c);
            }
        }
    }

    links
}

/// Format content type for display
fn format_type_name(content_type: &str) -> String {
    content_type
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Print analysis result
fn print_result(result: &AnalysisResult, _verbose: bool) {
    // Header
    let title = result.title.as_deref().unwrap_or("Untitled");
    println!(
        "\n{}: {}",
        result.item_type.cyan().bold(),
        title.white().bold()
    );

    if let Some(status) = &result.status {
        println!("{}: {}", "Status".dimmed(), status);
    }

    println!();

    // Checks - always show all checks
    for check in &result.checks {
        let indicator = if check.passed && !check.warning {
            "✓".green().bold()
        } else if check.warning {
            "⚠".yellow().bold()
        } else {
            "✗".red().bold()
        };

        println!("{} {}", indicator, check.message);
    }

    // Suggestions
    if !result.suggestions.is_empty() {
        println!("\n{}", "Suggestions:".yellow().bold());
        for suggestion in &result.suggestions {
            println!("  - {}", suggestion);
        }
    }

    // Questions
    if !result.questions.is_empty() {
        println!("\n{}", "Interview questions:".cyan().bold());
        for question in &result.questions {
            println!("  - {}", question);
        }
    }

    println!();
}

/// Find a file by name across all contexts
fn find_file(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    let search_name = if name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{}.md", name)
    };

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && !is_hidden(&path)
                && let Some(found) = find_in_context(&path, &search_name, name)
            {
                return Some(found);
            }
        }
    }

    None
}

fn find_in_context(context_path: &Path, search_name: &str, id: &str) -> Option<std::path::PathBuf> {
    if let Ok(entries) = fs::read_dir(context_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = find_in_directory(&path, search_name, id) {
                    return Some(found);
                }
            } else if path.is_file() && matches_file(&path, search_name, id) {
                return Some(path);
            }
        }
    }

    None
}

fn find_in_directory(dir: &Path, search_name: &str, id: &str) -> Option<std::path::PathBuf> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && matches_file(&path, search_name, id) {
                return Some(path);
            }
        }
    }

    None
}

fn matches_file(path: &Path, search_name: &str, id: &str) -> bool {
    if let Some(filename) = path.file_name().and_then(|n| n.to_str())
        && filename == search_name
    {
        return true;
    }

    if path.extension().is_some_and(|e| e == "md")
        && let Ok(content) = fs::read_to_string(path)
        && let Some(rest) = content.strip_prefix("---")
        && let Some(end_idx) = rest.find("\n---")
    {
        let yaml_str = &rest[1..end_idx];
        for line in yaml_str.lines() {
            let line = line.trim();
            if let Some(id_value) = line.strip_prefix("id:") {
                let file_id = id_value.trim().trim_matches('"').trim_matches('\'');
                if file_id == id {
                    return true;
                }
            }
        }
    }

    false
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

/// Collect all known content IDs
fn collect_known_ids(root: &Path, known_ids: &mut HashSet<String>) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                collect_ids_from_context(&path, known_ids);
            }
        }
    }
}

fn collect_ids_from_context(context_path: &Path, known_ids: &mut HashSet<String>) {
    if let Ok(entries) = fs::read_dir(context_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_ids_from_directory(&path, known_ids);
            } else if path.is_file()
                && path.extension().is_some_and(|e| e == "md")
                && let Some(id) = extract_id(&path)
            {
                known_ids.insert(id);
            }
        }
    }
}

fn collect_ids_from_directory(dir: &Path, known_ids: &mut HashSet<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().is_some_and(|e| e == "md")
                && let Some(id) = extract_id(&path)
            {
                known_ids.insert(id);
            }
        }
    }
}

fn extract_id(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let rest = content.strip_prefix("---")?;
    let end_idx = rest.find("\n---")?;
    let yaml_str = &rest[1..end_idx];

    for line in yaml_str.lines() {
        let line = line.trim();
        if let Some(id_value) = line.strip_prefix("id:") {
            let id = id_value.trim().trim_matches('"').trim_matches('\'');
            return Some(id.to_string());
        }
    }

    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}
