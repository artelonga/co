//! Plan subcommand - create structured proposals from objectives
//!
//! The plan phase:
//! 1. Detects repo context from cwd or --repo flag
//! 2. Takes an objective from the user
//! 3. Gathers context through interview mode (user) or creates skeleton (agent)
//! 4. Creates a use-case file with structured user story and acceptance criteria
//! 5. Creates a GitHub issue and links it in the frontmatter

use co::config::GlobalConfig;
use colored::Colorize;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::Command;

/// Creation mode for the plan
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanMode {
    /// User writes everything manually via prompts
    Manual,
    /// Agent generates skeleton for LLM to complete
    Assisted,
}

/// A structured plan created from an objective
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Use-case ID (derived from objective)
    pub id: String,
    /// Requester role/persona
    pub requester: String,
    /// The objective statement
    pub objective: String,
    /// User story: As (role)
    pub as_role: String,
    /// User story: I need (feature)
    pub i_need: String,
    /// User story: So that (value)
    pub so_that: String,
    /// Acceptance criteria (list of checkboxes)
    pub acceptance_criteria: Vec<String>,
    /// Context (from file or inline)
    pub context: Option<String>,
    /// Repo alias for global tracking
    pub repo: String,
    /// GitHub issue number (set after creation)
    pub github: Option<u64>,
}

impl Plan {
    /// Generate markdown content for this plan
    pub fn to_markdown(&self, space: &str) -> String {
        let criteria_md: String = self
            .acceptance_criteria
            .iter()
            .map(|c| format!("- [ ] {}", c))
            .collect::<Vec<_>>()
            .join("\n");

        let context_section = self
            .context
            .as_ref()
            .map(|c| format!("\n## Context\n\n{}\n", c))
            .unwrap_or_default();

        let github_line = self
            .github
            .map(|n| format!("github: {}\n", n))
            .unwrap_or_default();

        format!(
            r#"---
schema_version: 2
language: en
space: {}
type: use-case
id: {}
status: todo
requester: {}
repo: {}
{}---

# {}
{}
## User Story

**AS** {}
**I NEED** {}
**TO** {}

## Acceptance Criteria

{}
"#,
            space,
            self.id,
            self.requester,
            self.repo,
            github_line,
            self.objective,
            context_section,
            self.as_role,
            self.i_need,
            self.so_that,
            criteria_md
        )
    }

    /// Generate the file path for this plan
    pub fn file_path(&self, space: &str) -> String {
        format!("{}/use-cases/{}.md", space, self.id)
    }
}

/// Generate an ID from an objective string
pub fn generate_id(objective: &str) -> String {
    objective
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("-")
}

/// Run the plan command
pub fn run(
    objective: &str,
    space: Option<&str>,
    repo_alias: Option<&str>,
    context_file: Option<&str>,
) {
    let space = space.unwrap_or("work");

    // Detect repo context
    let config = GlobalConfig::load();
    let repo = match repo_alias {
        Some(alias) => {
            // Validate the provided alias exists
            match config.get_by_alias(alias) {
                Some(r) => r.clone(),
                None => {
                    eprintln!(
                        "{} Repo '{}' not found. Use `co repo list` to see registered repos.",
                        "error:".red().bold(),
                        alias
                    );
                    std::process::exit(1);
                }
            }
        }
        None => {
            // Detect from current directory
            match config.detect_current_space() {
                Some(r) => r.clone(),
                None => {
                    eprintln!(
                        "{} Not in a registered repo. Use `co repo add . --alias <name>` to register.",
                        "error:".red().bold()
                    );
                    std::process::exit(1);
                }
            }
        }
    };

    let space_path = repo.path.join(space);

    // Verify space exists
    if !space_path.exists() {
        eprintln!(
            "{} Space '{}' does not exist in {}. Create it with: co init {}",
            "error:".red().bold(),
            space,
            repo.alias,
            space
        );
        std::process::exit(1);
    }

    println!("{}", "Plan Phase".bold().cyan());
    println!("{}", "─".repeat(50));
    println!();
    println!("{}: {}", "Objective".bold(), objective);
    println!("{}: {}", "Repo".dimmed(), repo.alias);

    // Load context if specified
    let context = context_file.and_then(|path| match fs::read_to_string(path) {
        Ok(content) => {
            println!("{}: {}", "Context from".dimmed(), path);
            Some(content)
        }
        Err(e) => {
            eprintln!(
                "{} Failed to read context file {}: {}",
                "warning:".yellow().bold(),
                path,
                e
            );
            None
        }
    });

    println!();

    // Select mode
    let mode = prompt_mode();

    // Create plan based on mode
    let mut plan = match mode {
        PlanMode::Manual => create_manual_plan(objective, &repo.alias, context),
        PlanMode::Assisted => create_assisted_plan(objective, &repo.alias, context),
    };

    // Create use-cases directory
    let use_cases_dir = space_path.join("use-cases");
    if let Err(e) = fs::create_dir_all(&use_cases_dir) {
        eprintln!(
            "{} Failed to create directory: {}",
            "error:".red().bold(),
            e
        );
        std::process::exit(1);
    }

    // Create GitHub issue
    println!();
    println!("{}", "Creating GitHub issue...".dimmed());

    let issue_body = format!(
        "## User Story\n\n**AS** {}\n**I NEED** {}\n**TO** {}\n\n## Acceptance Criteria\n\n{}",
        plan.as_role,
        plan.i_need,
        plan.so_that,
        plan.acceptance_criteria
            .iter()
            .map(|c| format!("- [ ] {}", c))
            .collect::<Vec<_>>()
            .join("\n")
    );

    match create_github_issue(&plan.objective, &issue_body, &repo.path) {
        Ok(issue_number) => {
            plan.github = Some(issue_number);
            println!("  {} GitHub issue #{} created", "✓".green(), issue_number);
        }
        Err(e) => {
            eprintln!(
                "{} Failed to create GitHub issue: {}",
                "warning:".yellow().bold(),
                e
            );
            eprintln!("{}", "Continuing without GitHub link...".dimmed());
        }
    }

    // Write the plan file
    let file_path = plan.file_path(space);
    let full_path = repo.path.join(&file_path);
    let markdown = plan.to_markdown(space);

    // Check if file exists
    let exists = full_path.exists();
    if exists {
        println!(
            "{} File {} exists and will be updated",
            "warning:".yellow().bold(),
            file_path
        );
    }

    match fs::write(&full_path, markdown) {
        Ok(()) => {
            let symbol = if mode == PlanMode::Assisted {
                "◇"
            } else {
                "✓"
            };
            let action = if exists { "Updated" } else { "Created" };
            println!();
            println!("{}", "─".repeat(50));
            println!("{} {} {}", symbol.green().bold(), action.green(), file_path);

            if mode == PlanMode::Assisted {
                println!();
                println!(
                    "{}",
                    "Skeleton created. Fill in the sections and review.".yellow()
                );
            } else {
                println!();
                println!("{}", "Review the plan. When ready, execute with:".dimmed());
            }
            println!("  {} {}", "co conduct execute".cyan(), plan.id);
        }
        Err(e) => {
            eprintln!("{} Failed to write file: {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

/// Create a GitHub issue and return the issue number
fn create_github_issue(title: &str, body: &str, repo_path: &Path) -> Result<u64, String> {
    let output = Command::new("gh")
        .args(["issue", "create", "--title", title, "--body", body])
        .current_dir(repo_path)
        .output()
        .map_err(|e| format!("Failed to run gh: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.to_string());
    }

    // Parse issue URL to extract number
    // gh issue create outputs: https://github.com/owner/repo/issues/123
    let stdout = String::from_utf8_lossy(&output.stdout);
    let url = stdout.trim();

    url.rsplit('/')
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("Could not parse issue number from: {}", url))
}

/// Prompt for mode selection
fn prompt_mode() -> PlanMode {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    println!("{}", "How would you like to create the plan?".bold());
    println!("  1. {} - Write all details manually", "Manual".cyan());
    println!(
        "  2. {} - Generate skeleton for AI to complete",
        "Assisted".cyan()
    );
    print!("{}", "Choice [1/2]: ".dimmed());
    let _ = stdout.flush();

    let mut line = String::new();
    match stdin.lock().read_line(&mut line) {
        Ok(_) => match line.trim() {
            "2" | "assisted" | "a" => PlanMode::Assisted,
            _ => PlanMode::Manual,
        },
        Err(_) => PlanMode::Manual,
    }
}

/// Create a plan manually through prompts
fn create_manual_plan(objective: &str, repo: &str, context: Option<String>) -> Plan {
    let id = generate_id(objective);

    println!();
    println!("{}", "Interview Mode".bold().yellow());
    println!();

    let requester = prompt("Requester (role/persona)", "user");

    println!();
    println!("{}", "User Story:".bold());
    let as_role = prompt("AS A", "");
    let i_need = prompt("I NEED", "");
    let so_that = prompt("SO THAT", "");

    println!();
    println!("{}", "Acceptance Criteria (empty line to finish):".bold());
    let criteria = prompt_list("  - ");

    Plan {
        id,
        requester,
        objective: objective.to_string(),
        as_role,
        i_need,
        so_that,
        acceptance_criteria: if criteria.is_empty() {
            vec!["Meets the stated objective".to_string()]
        } else {
            criteria
        },
        context,
        repo: repo.to_string(),
        github: None,
    }
}

/// Create an assisted plan (skeleton for LLM)
fn create_assisted_plan(objective: &str, repo: &str, context: Option<String>) -> Plan {
    let id = generate_id(objective);

    Plan {
        id,
        requester: "[REQUESTER_ROLE]".to_string(),
        objective: objective.to_string(),
        as_role: "[WHO_BENEFITS]".to_string(),
        i_need: "[WHAT_FEATURE]".to_string(),
        so_that: "[WHAT_VALUE]".to_string(),
        acceptance_criteria: vec![
            "[CRITERION_1]".to_string(),
            "[CRITERION_2]".to_string(),
            "[CRITERION_3]".to_string(),
        ],
        context,
        repo: repo.to_string(),
        github: None,
    }
}

/// Prompt for input with a label and default value
fn prompt(label: &str, default: &str) -> String {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    if default.is_empty() {
        print!("{}: ", label.cyan());
    } else {
        print!("{} [{}]: ", label.cyan(), default.dimmed());
    }
    let _ = stdout.flush();

    let mut line = String::new();
    match stdin.lock().read_line(&mut line) {
        Ok(_) => {
            let value = line.trim();
            if value.is_empty() {
                default.to_string()
            } else {
                value.to_string()
            }
        }
        Err(_) => default.to_string(),
    }
}

/// Prompt for a list of items
fn prompt_list(prefix: &str) -> Vec<String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut items = Vec::new();

    loop {
        print!("{}", prefix.dimmed());
        let _ = stdout.flush();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(_) => {
                let value = line.trim();
                if value.is_empty() {
                    break;
                }
                items.push(value.to_string());
            }
            Err(_) => break,
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_id_creates_slug() {
        let id = generate_id("Write a recommendation letter");
        assert!(!id.is_empty());
        assert!(!id.contains(' '));
        assert!(id.chars().all(|c| c.is_alphanumeric() || c == '-'));
    }

    #[test]
    fn test_generate_id_truncates_long_objectives() {
        let id = generate_id("This is a very long objective with many words");
        let parts: Vec<&str> = id.split('-').collect();
        assert!(parts.len() <= 4);
    }

    #[test]
    fn test_generate_id_handles_special_chars() {
        let id = generate_id("User's Application Online!");
        assert!(!id.contains('\''));
        assert!(!id.contains('!'));
        assert!(id.contains("user"));
    }

    #[test]
    fn test_plan_to_markdown_contains_required_fields() {
        let plan = Plan {
            id: "test-plan".to_string(),
            requester: "tester".to_string(),
            objective: "Test objective".to_string(),
            as_role: "a tester".to_string(),
            i_need: "a test".to_string(),
            so_that: "I can verify".to_string(),
            acceptance_criteria: vec!["First criterion".to_string()],
            context: None,
            repo: "test-repo".to_string(),
            github: None,
        };

        let md = plan.to_markdown("work");

        assert!(md.contains("type: use-case"));
        assert!(md.contains("id: test-plan"));
        assert!(md.contains("status: todo"));
        assert!(md.contains("repo: test-repo"));
        assert!(md.contains("**AS**"));
        assert!(md.contains("**I NEED**"));
        assert!(md.contains("**TO**"));
        assert!(md.contains("- [ ]"));
    }

    #[test]
    fn test_plan_to_markdown_with_github_number() {
        let plan = Plan {
            id: "test-plan".to_string(),
            requester: "tester".to_string(),
            objective: "Test".to_string(),
            as_role: "user".to_string(),
            i_need: "test".to_string(),
            so_that: "verify".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            context: None,
            repo: "my-repo".to_string(),
            github: Some(42),
        };

        let md = plan.to_markdown("work");
        assert!(md.contains("github: 42"));
    }

    #[test]
    fn test_plan_file_path_format() {
        let plan = Plan {
            id: "my-plan".to_string(),
            requester: String::new(),
            objective: String::new(),
            as_role: String::new(),
            i_need: String::new(),
            so_that: String::new(),
            acceptance_criteria: vec![],
            context: None,
            repo: String::new(),
            github: None,
        };

        let path = plan.file_path("work");
        assert!(path.starts_with("work/"));
        assert!(path.ends_with(".md"));
        assert!(path.contains("use-cases"));
    }

    #[test]
    fn test_plan_with_context_includes_context_section() {
        let plan = Plan {
            id: "test".to_string(),
            requester: "user".to_string(),
            objective: "Test".to_string(),
            as_role: "user".to_string(),
            i_need: "test".to_string(),
            so_that: "verify".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            context: Some("Relevant context here".to_string()),
            repo: "repo".to_string(),
            github: None,
        };

        let md = plan.to_markdown("work");
        assert!(md.contains("## Context"));
        assert!(md.contains("Relevant context here"));
    }
}
