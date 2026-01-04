//! GitHub issue commands - `co gh issue`

use co::{GhCli, GithubIssue};
use colored::Colorize;
use comfy_table::{Cell, Table, presets::UTF8_FULL_CONDENSED};

/// Issue subcommand action
pub enum IssueAction {
    /// List issues
    List {
        state: String,
        label: Option<String>,
        limit: u32,
    },
    /// Show a specific issue
    Show { number: u64 },
}

/// Run issue subcommand
pub fn run(action: IssueAction) {
    match action {
        IssueAction::List {
            state,
            label,
            limit,
        } => list_issues(&state, label.as_deref(), limit),
        IssueAction::Show { number } => show_issue(number),
    }
}

/// List issues
fn list_issues(state: &str, label: Option<&str>, limit: u32) {
    let labels: Option<Vec<&str>> = label.map(|l| vec![l]);
    let label_refs = labels.as_deref();

    match GhCli::issue_list(state, label_refs, limit) {
        Ok(issues) => {
            if issues.is_empty() {
                println!("{}", "No issues found.".dimmed());
                return;
            }
            print_issues_table(&issues);
        }
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

/// Show a single issue
fn show_issue(number: u64) {
    match GhCli::issue_view(number) {
        Ok(issue) => print_issue_details(&issue),
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

/// Print issues in a table format
fn print_issues_table(issues: &[GithubIssue]) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(vec![
        Cell::new("#").fg(comfy_table::Color::Cyan),
        Cell::new("Title"),
        Cell::new("State"),
        Cell::new("Type"),
        Cell::new("Assignee"),
    ]);

    for issue in issues {
        let state_cell = if issue.state == "open" {
            Cell::new("open").fg(comfy_table::Color::Green)
        } else {
            Cell::new("closed").fg(comfy_table::Color::Red)
        };

        let type_str = issue.content_type().unwrap_or_else(|| "-".to_string());
        let assignee = issue.first_assignee().unwrap_or("-");

        // Truncate long titles
        let title = if issue.title.len() > 50 {
            format!("{}...", &issue.title[..47])
        } else {
            issue.title.clone()
        };

        table.add_row(vec![
            Cell::new(issue.number),
            Cell::new(title),
            state_cell,
            Cell::new(type_str),
            Cell::new(assignee),
        ]);
    }

    println!("{table}");
}

/// Print detailed issue information
fn print_issue_details(issue: &GithubIssue) {
    println!("{}", format!("Issue #{}", issue.number).bold().cyan());
    println!("{}", "═".repeat(60));

    // State badge
    let state_str = if issue.state == "open" {
        "● OPEN".green().to_string()
    } else {
        "○ CLOSED".red().to_string()
    };
    println!("{}", state_str);
    println!();

    // Title
    println!("{}", issue.title.bold());
    println!();

    // Metadata
    if let Some(content_type) = issue.content_type() {
        println!("  {:<12} {}", "Type:".dimmed(), content_type);
    }
    if let Some(priority) = issue.priority() {
        println!("  {:<12} {}", "Priority:".dimmed(), priority);
    }
    if let Some(assignee) = issue.first_assignee() {
        println!("  {:<12} {}", "Assignee:".dimmed(), assignee);
    }

    // Labels
    if !issue.labels.is_empty() {
        let labels: Vec<_> = issue.labels.iter().map(|l| l.name.as_str()).collect();
        println!("  {:<12} {}", "Labels:".dimmed(), labels.join(", "));
    }

    // URL
    if !issue.url.is_empty() {
        println!("  {:<12} {}", "URL:".dimmed(), issue.url);
    }

    // Body
    if !issue.body.is_empty() {
        println!();
        println!("{}", "─".repeat(60));
        println!("{}", issue.body);
    }
}
