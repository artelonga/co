//! Locate command - unified content search and index management
//!
//! Searches content by frontmatter fields and/or body text.
//!
//! Query patterns:
//! - `field:value` - Filter by frontmatter field
//! - Plain text - Full-text search in body
//!
//! Federation filters (--in):
//! - `@groupname` - Query repos in a group
//! - `#tagname` - Query repos with a tag
//! - `alias` - Query a specific registered repo
//!
//! Subcommands:
//! - `build` - Build search index
//! - `update` - Update index incrementally
//! - `stats` - Show index statistics
//!
//! Examples:
//!   co locate status:todo           # Filter by frontmatter
//!   co locate "important meeting"   # Full-text search
//!   co locate status:todo meeting   # Combined filter + search
//!   co locate private status:todo   # Context + filter
//!   co locate --in @work status:todo  # Federated query across work group
//!   co locate build                 # Build search index

pub mod build;
pub mod stats;
pub mod update;

use co::config::{resolve_repos, GlobalConfig, GroupsConfig};
use colored::Colorize;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Content item with context information
struct ContentItem {
    repo_alias: Option<String>,
    context: String,
    id: String,
}

/// Frontmatter for filtering (captures all fields)
#[derive(Debug, Deserialize)]
struct FilterableFrontmatter {
    #[serde(flatten)]
    fields: HashMap<String, serde_yaml::Value>,
}

/// Run the unified locate command
pub fn run(query: &[String], context_filter: Option<&str>) {
    // Check if this is a federated query (@group, #tag, or registered repo alias)
    if let Some(filter) = context_filter {
        if filter.starts_with('@') || filter.starts_with('#') || is_registered_repo(filter) {
            run_federated(query, filter);
            return;
        }
    }

    // Check if first argument is a positional context
    // It's a context if: no colon, no spaces, and there are subsequent filter args (with colons)
    // This distinguishes `co locate private status:todo` from `co locate "search term"`
    let has_subsequent_filters = query.len() > 1 && query[1..].iter().any(|a| a.contains(':'));

    let (positional_context, query_args): (Option<&str>, &[String]) = if !query.is_empty()
        && !query[0].contains(':')
        && !query[0].contains(' ')
        && has_subsequent_filters
    {
        // First arg looks like a context name (followed by filters)
        if !is_context_dir(&query[0]) {
            eprintln!("{} Context '{}' not found", "error:".red().bold(), query[0]);
            std::process::exit(1);
        }
        (Some(&query[0]), &query[1..])
    } else if !query.is_empty()
        && !query[0].contains(':')
        && !query[0].contains(' ')
        && is_context_dir(&query[0])
    {
        // First arg is an existing context (even without subsequent filters)
        (Some(&query[0]), &query[1..])
    } else {
        (None, query)
    };

    // Separate field filters (field:value) from search terms (plain text)
    let mut field_filters: Vec<(String, String)> = Vec::new();
    let mut search_terms: Vec<String> = Vec::new();

    for arg in query_args {
        if let Some(colon_pos) = arg.find(':') {
            let field = &arg[..colon_pos];
            let value = &arg[colon_pos + 1..];
            field_filters.push((field.to_string(), value.to_string()));
        } else {
            search_terms.push(arg.to_lowercase());
        }
    }

    // Combine --in flag with positional context (--in takes precedence)
    let effective_context = context_filter.or(positional_context);

    // Parse context filter (comma-separated)
    let contexts: Option<Vec<&str>> = effective_context.map(|s| s.split(',').collect());

    // Collect all matching content
    let results = find_content(&field_filters, &search_terms, contexts.as_deref());

    if results.is_empty() {
        println!("{}", "No results found".yellow());
        return;
    }

    // Display results with context prefix
    for item in results {
        println!(
            "{} {}",
            format!("[{}]", item.context).cyan(),
            item.id.white()
        );
    }
}

/// Check if a filter matches a registered repository alias
fn is_registered_repo(filter: &str) -> bool {
    let global = GlobalConfig::load();
    global.get_by_alias(filter).is_some()
}

/// Run a federated query across multiple repositories
fn run_federated(query: &[String], filter: &str) {
    let global = GlobalConfig::load();
    let groups = GroupsConfig::load();

    let repos = resolve_repos(filter, &global, &groups);

    if repos.is_empty() {
        if filter.starts_with('@') {
            eprintln!(
                "{} No repositories match group '{}'",
                "error:".red().bold(),
                &filter[1..]
            );
        } else if filter.starts_with('#') {
            eprintln!(
                "{} No repositories have tag '{}'",
                "error:".red().bold(),
                &filter[1..]
            );
        } else {
            eprintln!(
                "{} Repository '{}' not registered",
                "error:".red().bold(),
                filter
            );
        }
        std::process::exit(1);
    }

    // Parse query
    let mut field_filters: Vec<(String, String)> = Vec::new();
    let mut search_terms: Vec<String> = Vec::new();

    for arg in query {
        if let Some(colon_pos) = arg.find(':') {
            let field = &arg[..colon_pos];
            let value = &arg[colon_pos + 1..];
            field_filters.push((field.to_string(), value.to_string()));
        } else {
            search_terms.push(arg.to_lowercase());
        }
    }

    let mut all_results = Vec::new();

    for repo in &repos {
        if !repo.path.exists() {
            eprintln!(
                "{} Repository '{}' path not found: {}",
                "warning:".yellow().bold(),
                repo.alias,
                repo.path.display()
            );
            continue;
        }

        // Search in this repo
        let results = find_content_in_repo(&repo.path, &repo.alias, &field_filters, &search_terms);
        all_results.extend(results);
    }

    if all_results.is_empty() {
        println!("{}", "No results found".yellow());
        return;
    }

    // Display results with repo prefix
    for item in all_results {
        let prefix = if let Some(ref alias) = item.repo_alias {
            format!("[{}:{}]", alias, item.context)
        } else {
            format!("[{}]", item.context)
        };
        println!("{} {}", prefix.cyan(), item.id.white());
    }
}

/// Find content in a specific repository
fn find_content_in_repo(
    repo_root: &Path,
    repo_alias: &str,
    filters: &[(String, String)],
    search_terms: &[String],
) -> Vec<ContentItem> {
    let mut results = Vec::new();

    // Iterate through all directories (potential contexts)
    if let Ok(entries) = fs::read_dir(repo_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                let context_name = path.file_name().unwrap().to_string_lossy().to_string();

                // Skip common non-content directories
                if context_name == "target" || context_name == "node_modules" {
                    continue;
                }

                // Search in type subdirectories
                if let Ok(type_dirs) = fs::read_dir(&path) {
                    for type_dir in type_dirs.flatten() {
                        let type_path = type_dir.path();
                        if type_path.is_dir() {
                            search_directory_federated(
                                &type_path,
                                repo_alias,
                                &context_name,
                                filters,
                                search_terms,
                                &mut results,
                            );
                        }
                    }
                }

                // Also search directly in the context root
                search_directory_federated(
                    &path,
                    repo_alias,
                    &context_name,
                    filters,
                    search_terms,
                    &mut results,
                );
            }
        }
    }

    results
}

/// Search a directory for matching content files (federated version)
fn search_directory_federated(
    dir: &Path,
    repo_alias: &str,
    context: &str,
    filters: &[(String, String)],
    search_terms: &[String],
    results: &mut Vec<ContentItem>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "md") {
                if let Some(mut item) = check_file(&path, context, filters, search_terms) {
                    item.repo_alias = Some(repo_alias.to_string());
                    results.push(item);
                }
            }
        }
    }
}

/// Find content matching filters and/or search terms
fn find_content(
    filters: &[(String, String)],
    search_terms: &[String],
    contexts: Option<&[&str]>,
) -> Vec<ContentItem> {
    let mut results = Vec::new();
    let current_dir = Path::new(".");

    // Iterate through all directories (potential contexts)
    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_hidden(&path) {
                let context_name = path.file_name().unwrap().to_string_lossy().to_string();

                // Skip if context filter is set and this context is not in the list
                if let Some(allowed_contexts) = contexts {
                    if !allowed_contexts.contains(&context_name.as_str()) {
                        continue;
                    }
                }

                // Search in type subdirectories (tasks/, definitions/, etc.)
                if let Ok(type_dirs) = fs::read_dir(&path) {
                    for type_dir in type_dirs.flatten() {
                        let type_path = type_dir.path();
                        if type_path.is_dir() {
                            search_directory(
                                &type_path,
                                &context_name,
                                filters,
                                search_terms,
                                &mut results,
                            );
                        }
                    }
                }

                // Also search directly in the context root
                search_directory(&path, &context_name, filters, search_terms, &mut results);
            }
        }
    }

    results
}

/// Search a directory for matching content files
fn search_directory(
    dir: &Path,
    context: &str,
    filters: &[(String, String)],
    search_terms: &[String],
    results: &mut Vec<ContentItem>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|e| e == "md") {
                if let Some(item) = check_file(&path, context, filters, search_terms) {
                    results.push(item);
                }
            }
        }
    }
}

/// Check if a file matches the filters and search terms
fn check_file(
    path: &Path,
    context: &str,
    filters: &[(String, String)],
    search_terms: &[String],
) -> Option<ContentItem> {
    let content = fs::read_to_string(path).ok()?;

    // Extract frontmatter
    if !content.starts_with("---") {
        return None;
    }

    let end_idx = content[3..].find("\n---")?;
    let yaml_str = &content[4..end_idx + 3];
    let body_start = end_idx + 7; // Skip past "\n---\n"
    let body = if body_start < content.len() {
        &content[body_start..]
    } else {
        ""
    };

    let frontmatter: FilterableFrontmatter = serde_yaml::from_str(yaml_str).ok()?;

    // Convert fields to string map for comparison
    let mut string_fields: HashMap<String, String> = HashMap::new();
    for (key, value) in &frontmatter.fields {
        if let Some(s) = value.as_str() {
            string_fields.insert(key.clone(), s.to_string());
        }
    }

    // Check all frontmatter filters match
    for (field, value) in filters {
        match string_fields.get(field) {
            Some(v) if v == value => continue,
            _ => return None,
        }
    }

    // Check all search terms are found in body
    let body_lower = body.to_lowercase();
    for term in search_terms {
        if !body_lower.contains(term) {
            return None;
        }
    }

    // Get ID from frontmatter or filename
    let id = string_fields.get("id").cloned().unwrap_or_else(|| {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });

    Some(ContentItem {
        repo_alias: None,
        context: context.to_string(),
        id,
    })
}

/// Check if a path is hidden (starts with .)
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

/// Check if a name corresponds to an existing context directory
fn is_context_dir(name: &str) -> bool {
    let path = Path::new(name);
    path.is_dir() && !is_hidden(path)
}
