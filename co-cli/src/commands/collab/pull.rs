//! Pull GitHub issues to local markdown files - `co collab pull`

use co::{GhCli, MappedWorkItem};
use colored::Colorize;
use std::fs;
use std::path::Path;

/// Run the pull command
pub fn run(all: bool, numbers: Vec<u64>) {
    if !all && numbers.is_empty() {
        eprintln!(
            "{} Specify issue numbers or use --all to pull all open issues",
            "error:".red().bold()
        );
        std::process::exit(1);
    }

    let issues = if all {
        // Fetch all open issues
        match GhCli::issue_list("open", None, 100) {
            Ok(issues) => issues,
            Err(e) => {
                eprintln!("{} {}", "error:".red().bold(), e);
                std::process::exit(1);
            }
        }
    } else {
        // Fetch specific issues
        let mut issues = Vec::new();
        for number in &numbers {
            match GhCli::issue_view(*number) {
                Ok(issue) => issues.push(issue),
                Err(e) => {
                    eprintln!(
                        "{} Failed to fetch issue #{}: {}",
                        "warning:".yellow().bold(),
                        number,
                        e
                    );
                }
            }
        }
        issues
    };

    if issues.is_empty() {
        println!("{}", "No issues to pull.".dimmed());
        return;
    }

    println!(
        "{} {} issue(s) from GitHub",
        "Pulling".green().bold(),
        issues.len()
    );
    println!("{}", "─".repeat(50));

    let space = "work";
    let language = "en";
    let mut created = 0;
    let mut updated = 0;
    let mut skipped = 0;

    for issue in &issues {
        let mapped = MappedWorkItem::from_github_issue(issue);
        let file_path = mapped.file_path(space);
        let path = Path::new(&file_path);

        // Ensure directory exists
        if let Some(parent) = path.parent()
            && let Err(e) = fs::create_dir_all(parent)
        {
            eprintln!(
                "{} Failed to create directory {}: {}",
                "error:".red().bold(),
                parent.display(),
                e
            );
            skipped += 1;
            continue;
        }

        // Check if file exists
        let exists = path.exists();
        let action_str = if exists { "Updated" } else { "Created" };

        // Write the markdown file
        let markdown = mapped.to_markdown(space, language);
        match fs::write(path, markdown) {
            Ok(()) => {
                println!(
                    "  {} {} ({})",
                    if exists {
                        "↻".yellow()
                    } else {
                        "✓".green()
                    },
                    file_path,
                    action_str.dimmed()
                );
                if exists {
                    updated += 1;
                } else {
                    created += 1;
                }
            }
            Err(e) => {
                eprintln!(
                    "{} Failed to write {}: {}",
                    "error:".red().bold(),
                    file_path,
                    e
                );
                skipped += 1;
            }
        }
    }

    println!("{}", "─".repeat(50));
    println!(
        "{}: {} created, {} updated, {} skipped",
        "Summary".bold(),
        created.to_string().green(),
        updated.to_string().yellow(),
        if skipped > 0 {
            skipped.to_string().red()
        } else {
            skipped.to_string().dimmed()
        }
    );
}
