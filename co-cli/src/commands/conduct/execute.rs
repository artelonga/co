//! Execute subcommand - drive plans through git workflow states
//!
//! Status transitions:
//! - todo → in-progress: Create git branch
//! - in-progress → review: Detect PR creation
//! - review → done: Detect PR merge

use co::config::GlobalConfig;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Plan status in the git workflow
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Todo,
    InProgress,
    Review,
    Done,
}

impl PlanStatus {
    /// Parse status from frontmatter string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "todo" => Some(Self::Todo),
            "in-progress" | "in_progress" => Some(Self::InProgress),
            "review" => Some(Self::Review),
            "done" => Some(Self::Done),
            _ => None,
        }
    }

    /// Convert to frontmatter string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in-progress",
            Self::Review => "review",
            Self::Done => "done",
        }
    }
}

/// Loaded plan data from file
#[derive(Debug)]
pub struct LoadedPlan {
    pub id: String,
    pub status: PlanStatus,
    pub branch: Option<String>,
    pub pr: Option<u64>,
    pub file_path: PathBuf,
    pub content: String,
}

impl LoadedPlan {
    /// Load a plan from a file
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

        // Parse frontmatter
        let lines: Vec<&str> = content.lines().collect();
        if lines.first() != Some(&"---") {
            return Err("Invalid frontmatter: missing opening ---".to_string());
        }

        let end_idx = lines
            .iter()
            .skip(1)
            .position(|l| *l == "---")
            .ok_or("Invalid frontmatter: missing closing ---")?
            + 1;

        let mut id = None;
        let mut status = None;
        let mut branch = None;
        let mut pr = None;

        for line in &lines[1..end_idx] {
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "id" => id = Some(value.to_string()),
                    "status" => status = PlanStatus::from_str(value),
                    "branch" => branch = Some(value.to_string()),
                    "pr" => pr = value.parse().ok(),
                    _ => {}
                }
            }
        }

        let id = id.ok_or("Missing id in frontmatter")?;
        let status = status.ok_or("Missing or invalid status in frontmatter")?;

        Ok(Self {
            id,
            status,
            branch,
            pr,
            file_path: path.to_path_buf(),
            content,
        })
    }

    /// Update the status in the file content and save
    pub fn update_status(&mut self, new_status: PlanStatus) -> Result<(), String> {
        let old_status_line = format!("status: {}", self.status.as_str());
        let new_status_line = format!("status: {}", new_status.as_str());

        self.content = self.content.replace(&old_status_line, &new_status_line);
        self.status = new_status;

        fs::write(&self.file_path, &self.content)
            .map_err(|e| format!("Failed to write {}: {}", self.file_path.display(), e))
    }

    /// Add or update a frontmatter field
    pub fn set_field(&mut self, key: &str, value: &str) -> Result<(), String> {
        let lines: Vec<&str> = self.content.lines().collect();
        let end_idx = lines
            .iter()
            .skip(1)
            .position(|l| *l == "---")
            .ok_or("Invalid frontmatter")?
            + 1;

        // Check if field exists
        let field_line = format!("{}: ", key);
        let mut found = false;
        let mut new_lines: Vec<String> = Vec::new();

        for (i, line) in lines.iter().enumerate() {
            if i > 0 && i < end_idx && line.starts_with(&field_line) {
                new_lines.push(format!("{}: {}", key, value));
                found = true;
            } else if i == end_idx && !found {
                // Insert before closing ---
                new_lines.push(format!("{}: {}", key, value));
                new_lines.push(line.to_string());
            } else {
                new_lines.push(line.to_string());
            }
        }

        self.content = new_lines.join("\n");
        fs::write(&self.file_path, &self.content)
            .map_err(|e| format!("Failed to write {}: {}", self.file_path.display(), e))
    }
}

/// Find a plan file by ID in the repo
fn find_plan(id: &str, repo_path: &Path) -> Result<PathBuf, String> {
    // Search common space directories for use-cases
    let spaces = ["work", "private", "shared"];
    let filename = format!("{}.md", id);

    for space in &spaces {
        let path = repo_path.join(space).join("use-cases").join(&filename);
        if path.exists() {
            return Ok(path);
        }
    }

    // Also try direct path in case id includes path
    let direct_path = repo_path.join(&filename);
    if direct_path.exists() {
        return Ok(direct_path);
    }

    Err(format!(
        "Plan '{}' not found. Searched in work/use-cases/, private/use-cases/, shared/use-cases/",
        id
    ))
}

/// Run the execute command
pub fn run(id: &str, repo_alias: Option<&str>) {
    // Detect repo context
    let config = GlobalConfig::load();
    let repo = match repo_alias {
        Some(alias) => match config.get_by_alias(alias) {
            Some(r) => r.clone(),
            None => {
                eprintln!(
                    "{} Repo '{}' not found. Use `co repo list` to see registered repos.",
                    "error:".red().bold(),
                    alias
                );
                std::process::exit(1);
            }
        },
        None => match config.detect_current_space() {
            Some(r) => r.clone(),
            None => {
                eprintln!(
                    "{} Not in a registered repo. Use `co repo add . --alias <name>` to register.",
                    "error:".red().bold()
                );
                std::process::exit(1);
            }
        },
    };

    // Find the plan file
    let plan_path = match find_plan(id, &repo.path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    };

    // Load the plan
    let mut plan = match LoadedPlan::load(&plan_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    };

    println!("{}", "Execute Phase".bold().cyan());
    println!("{}", "─".repeat(50));
    println!();
    println!("{}: {}", "Plan".bold(), plan.id);
    println!("{}: {}", "Status".bold(), plan.status.as_str());
    println!("{}: {}", "Repo".dimmed(), repo.alias);
    println!();

    // Execute based on current status
    match plan.status {
        PlanStatus::Todo => execute_todo_to_in_progress(&mut plan, &repo.path),
        PlanStatus::InProgress => execute_in_progress_to_review(&mut plan, &repo.path),
        PlanStatus::Review => execute_review_to_done(&mut plan, &repo.path),
        PlanStatus::Done => {
            println!("{} Plan already complete.", "✓".green().bold());
        }
    }
}

/// Transition from todo to in-progress: create branch
fn execute_todo_to_in_progress(plan: &mut LoadedPlan, repo_path: &Path) {
    let branch_name = format!("feat/{}", plan.id);

    println!("Creating branch: {}", branch_name.cyan());

    // Create and checkout branch
    let output = Command::new("git")
        .args(["checkout", "-b", &branch_name])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            println!("  {} Branch created", "✓".green());

            // Update plan file
            if let Err(e) = plan.set_field("branch", &branch_name) {
                eprintln!(
                    "{} Failed to update branch field: {}",
                    "warning:".yellow(),
                    e
                );
            }

            if let Err(e) = plan.update_status(PlanStatus::InProgress) {
                eprintln!("{} Failed to update status: {}", "error:".red().bold(), e);
                std::process::exit(1);
            }

            println!("  {} Status: in-progress", "✓".green());
            println!();
            println!("{}", "─".repeat(50));
            println!(
                "{}",
                "Work on your implementation. When ready, create a PR.".dimmed()
            );
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            eprintln!(
                "{} Failed to create branch: {}",
                "error:".red().bold(),
                stderr.trim()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("{} Failed to run git: {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

/// Transition from in-progress to review: detect PR
fn execute_in_progress_to_review(plan: &mut LoadedPlan, repo_path: &Path) {
    println!("Checking for pull request...");

    let default_branch = format!("feat/{}", plan.id);
    let branch = plan.branch.as_deref().unwrap_or(&default_branch);

    // Check for PR on this branch
    let output = Command::new("gh")
        .args([
            "pr", "list", "--head", branch, "--json", "number", "--limit", "1",
        ])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // Parse JSON: [{"number":123}] or []
            if let Some(num) = extract_pr_number(&stdout) {
                println!("  {} PR #{} found", "✓".green(), num);

                // Update plan file
                if let Err(e) = plan.set_field("pr", &num.to_string()) {
                    eprintln!("{} Failed to update pr field: {}", "warning:".yellow(), e);
                }

                if let Err(e) = plan.update_status(PlanStatus::Review) {
                    eprintln!("{} Failed to update status: {}", "error:".red().bold(), e);
                    std::process::exit(1);
                }

                println!("  {} Status: review", "✓".green());
            } else {
                println!("  {} No PR found for branch '{}'", "○".yellow(), branch);
                println!();
                println!("{}", "Create a PR with: gh pr create".dimmed());
            }
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            eprintln!(
                "{} Failed to check PR: {}",
                "warning:".yellow().bold(),
                stderr.trim()
            );
        }
        Err(e) => {
            eprintln!("{} Failed to run gh: {}", "warning:".yellow().bold(), e);
        }
    }
}

/// Transition from review to done: detect merge
fn execute_review_to_done(plan: &mut LoadedPlan, repo_path: &Path) {
    println!("Checking PR status...");

    let pr_num = match plan.pr {
        Some(n) => n,
        None => {
            println!(
                "  {} No PR number recorded. Run execute again after PR is created.",
                "○".yellow()
            );
            return;
        }
    };

    // Check PR state
    let output = Command::new("gh")
        .args(["pr", "view", &pr_num.to_string(), "--json", "state"])
        .current_dir(repo_path)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            // Parse JSON: {"state":"MERGED"} or {"state":"OPEN"}
            if stdout.contains("MERGED") {
                println!("  {} PR #{} merged", "✓".green(), pr_num);

                if let Err(e) = plan.update_status(PlanStatus::Done) {
                    eprintln!("{} Failed to update status: {}", "error:".red().bold(), e);
                    std::process::exit(1);
                }

                println!("  {} Status: done", "✓".green());
                println!();
                println!("{}", "─".repeat(50));
                println!("{}", "Plan complete!".green().bold());
            } else if stdout.contains("CLOSED") {
                println!("  {} PR #{} was closed without merging", "✗".red(), pr_num);
            } else {
                println!("  {} PR #{} still open", "○".yellow(), pr_num);
                println!();
                println!("{}", "Merge the PR to complete this plan.".dimmed());
            }
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            eprintln!(
                "{} Failed to check PR status: {}",
                "warning:".yellow().bold(),
                stderr.trim()
            );
        }
        Err(e) => {
            eprintln!("{} Failed to run gh: {}", "warning:".yellow().bold(), e);
        }
    }
}

/// Extract PR number from gh JSON output
fn extract_pr_number(json: &str) -> Option<u64> {
    // Simple extraction: [{"number":123}]
    json.find("\"number\":").and_then(|i| {
        let rest = &json[i + 9..];
        let end = rest.find(|c: char| !c.is_ascii_digit())?;
        rest[..end].parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_status_from_str() {
        assert_eq!(PlanStatus::from_str("todo"), Some(PlanStatus::Todo));
        assert_eq!(
            PlanStatus::from_str("in-progress"),
            Some(PlanStatus::InProgress)
        );
        assert_eq!(
            PlanStatus::from_str("in_progress"),
            Some(PlanStatus::InProgress)
        );
        assert_eq!(PlanStatus::from_str("review"), Some(PlanStatus::Review));
        assert_eq!(PlanStatus::from_str("done"), Some(PlanStatus::Done));
        assert_eq!(PlanStatus::from_str("unknown"), None);
    }

    #[test]
    fn test_plan_status_as_str() {
        assert_eq!(PlanStatus::Todo.as_str(), "todo");
        assert_eq!(PlanStatus::InProgress.as_str(), "in-progress");
        assert_eq!(PlanStatus::Review.as_str(), "review");
        assert_eq!(PlanStatus::Done.as_str(), "done");
    }

    #[test]
    fn test_extract_pr_number() {
        assert_eq!(extract_pr_number(r#"[{"number":123}]"#), Some(123));
        assert_eq!(extract_pr_number(r#"[{"number":1}]"#), Some(1));
        assert_eq!(extract_pr_number(r#"[]"#), None);
        assert_eq!(
            extract_pr_number(r#"[{"number":456,"title":"test"}]"#),
            Some(456)
        );
    }
}
