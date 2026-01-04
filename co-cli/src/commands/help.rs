//! Help command - topic-based documentation
//!
//! Shows embedded documentation for concepts and workflows.

use crate::docs;
use colored::Colorize;

/// Run the help command
pub fn run(topic: Option<&str>) {
    match topic {
        Some(name) => show_topic(name),
        None => list_topics(),
    }
}

/// Show a specific topic
fn show_topic(name: &str) {
    match docs::get_topic(name) {
        Some(content) => {
            println!("{}", render_markdown(content));
        }
        None => {
            eprintln!("{} Unknown topic: {}", "error:".red().bold(), name);

            let similar = docs::find_similar(name);
            if !similar.is_empty() {
                eprintln!();
                eprintln!("{}", "Did you mean:".dimmed());
                for topic in similar {
                    eprintln!("  co help {}", topic.cyan());
                }
            } else {
                eprintln!();
                eprintln!("{}", "Available topics:".dimmed());
                for (topic, _) in docs::list_topics() {
                    eprintln!("  co help {}", topic.cyan());
                }
            }

            std::process::exit(1);
        }
    }
}

/// List all available topics
fn list_topics() {
    println!("{}", "CO Help".bold().cyan());
    println!("{}", "=".repeat(40));
    println!();
    println!("{}", "Conceptual guides:".bold());
    println!();

    for (topic, description) in docs::list_topics() {
        println!(
            "  {}  {}",
            format!("co help {}", topic).cyan(),
            description.dimmed()
        );
    }

    println!();
    println!("{}", "Command reference:".bold());
    println!();
    println!(
        "  {}  {}",
        "co --help".cyan(),
        "List all commands".dimmed()
    );
    println!(
        "  {}  {}",
        "co <command> --help".cyan(),
        "Detailed command syntax".dimmed()
    );
}

/// Simple markdown rendering for terminal
fn render_markdown(content: &str) -> String {
    let mut output = String::new();

    for line in content.lines() {
        if let Some(header) = line.strip_prefix("# ") {
            output.push_str(&format!("{}\n", header.bold().cyan()));
            output.push_str(&"=".repeat(40));
            output.push('\n');
        } else if let Some(header) = line.strip_prefix("## ") {
            output.push('\n');
            output.push_str(&format!("{}\n", header.bold()));
            output.push_str(&"-".repeat(30));
            output.push('\n');
        } else if let Some(header) = line.strip_prefix("### ") {
            output.push('\n');
            output.push_str(&format!("{}\n", header.bold()));
        } else if line.starts_with("```") {
            // Skip code fence markers, content will display as-is
        } else if line.starts_with("| ") {
            // Table row - display as-is
            output.push_str(line);
            output.push('\n');
        } else if let Some(item) = line.strip_prefix("- ") {
            output.push_str(&format!("  {} {}\n", "•".dimmed(), item));
        } else if let Some(item) = line.strip_prefix("* ") {
            output.push_str(&format!("  {} {}\n", "•".dimmed(), item));
        } else if line.starts_with("  ") && !line.trim().is_empty() {
            // Indented content (likely code)
            output.push_str(&format!("{}\n", line.dimmed()));
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_markdown_headers() {
        let input = "# Title\n## Section\n### Subsection";
        let output = render_markdown(input);
        assert!(output.contains("Title"));
        assert!(output.contains("Section"));
        assert!(output.contains("Subsection"));
    }

    #[test]
    fn test_render_markdown_lists() {
        let input = "- Item 1\n- Item 2";
        let output = render_markdown(input);
        assert!(output.contains("Item 1"));
        assert!(output.contains("Item 2"));
    }
}
