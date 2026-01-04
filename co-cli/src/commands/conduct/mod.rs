//! Conduct command - Plan & Execute workflow for objectives
//!
//! Two-phase workflow:
//! - Plan: Convert objectives into structured proposals with acceptance criteria
//! - Execute: Track implementation through git workflow states
//!
//! Status flow: todo → in-progress → review → done

pub mod execute;
pub mod plan;

/// Conduct subcommand actions
pub enum ConductAction {
    /// Create a structured plan from an objective
    Plan {
        /// The objective to plan
        objective: String,
        /// Target space within repo (default: work)
        space: Option<String>,
        /// Target repo alias (default: detect from cwd)
        repo: Option<String>,
        /// Context file path
        context: Option<String>,
    },
    /// Execute an approved plan through git workflow
    Execute {
        /// Plan ID to execute
        id: String,
        /// Target repo alias (default: detect from cwd or file's repo field)
        repo: Option<String>,
    },
}

/// Run the conduct command
pub fn run(action: ConductAction) {
    match action {
        ConductAction::Plan {
            objective,
            space,
            repo,
            context,
        } => plan::run(
            &objective,
            space.as_deref(),
            repo.as_deref(),
            context.as_deref(),
        ),
        ConductAction::Execute { id, repo } => execute::run(&id, repo.as_deref()),
    }
}
