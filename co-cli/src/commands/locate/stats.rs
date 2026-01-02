//! Stats command - `co locate stats`
//!
//! Shows index statistics including entry count and breakdown by scope/language.

use co::Index;
use colored::Colorize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Run the stats command
pub fn run() {
    println!("{}", "Index Statistics".bold().green());
    println!("{}", "─".repeat(40));

    let index_path = Path::new(".co/index.bin");

    // Load existing index
    let index = if index_path.exists() {
        match fs::read(index_path) {
            Ok(bytes) => match Index::from_bytes(&bytes) {
                Ok(idx) => idx,
                Err(_) => {
                    eprintln!("{}", "Index corrupted. Run `co locate build` to rebuild.".red());
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("{} Failed to read index: {}", "error:".red().bold(), e);
                std::process::exit(1);
            }
        }
    } else {
        println!("{}", "No index found. Run `co locate build` first.".yellow());
        return;
    };

    // Basic stats
    println!("  Version:      {}", index.version.to_string().cyan());
    println!("  Total:        {} entries", index.len().to_string().cyan());

    if index.is_empty() {
        return;
    }

    // Breakdown by scope
    let mut by_scope: HashMap<&str, usize> = HashMap::new();
    let mut by_type: HashMap<&str, usize> = HashMap::new();
    let mut by_language: HashMap<String, usize> = HashMap::new();

    for entry in index.entries.values() {
        *by_scope.entry(&entry.scope).or_insert(0) += 1;
        *by_type.entry(&entry.node_type).or_insert(0) += 1;
        let lang = entry.language.clone().unwrap_or_else(|| "unspecified".to_string());
        *by_language.entry(lang).or_insert(0) += 1;
    }

    // Display scope breakdown
    println!();
    println!("  {}", "By Scope:".bold());
    let mut scopes: Vec<_> = by_scope.iter().collect();
    scopes.sort_by(|a, b| b.1.cmp(a.1)); // Sort by count descending
    for (scope, count) in scopes {
        println!("    {:12} {}", scope, count.to_string().cyan());
    }

    // Display type breakdown
    println!();
    println!("  {}", "By Type:".bold());
    let mut types: Vec<_> = by_type.iter().collect();
    types.sort_by(|a, b| b.1.cmp(a.1));
    for (node_type, count) in types {
        println!("    {:12} {}", node_type, count.to_string().cyan());
    }

    // Display language breakdown
    println!();
    println!("  {}", "By Language:".bold());
    let mut languages: Vec<_> = by_language.iter().collect();
    languages.sort_by(|a, b| b.1.cmp(a.1));
    for (lang, count) in languages {
        println!("    {:12} {}", lang, count.to_string().cyan());
    }

    // Index file size
    if let Ok(metadata) = fs::metadata(index_path) {
        let size = metadata.len();
        let size_str = if size < 1024 {
            format!("{} bytes", size)
        } else if size < 1024 * 1024 {
            format!("{:.1} KB", size as f64 / 1024.0)
        } else {
            format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
        };
        println!();
        println!("  Index size:   {}", size_str.cyan());
    }
}
