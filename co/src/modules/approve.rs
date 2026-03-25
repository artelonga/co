use crate::context::{Context, Stage};
use crate::engine::LoggedToolRegistry;
use crate::module::Module;
use std::collections::HashMap;

/// Approve module: PR → review
/// Uses Claude to analyze the diff and gh to post a review.
pub struct ApproveModule;

impl Module for ApproveModule {
    fn name(&self) -> &str {
        "approve"
    }

    fn stage(&self) -> Stage {
        Stage::Approve
    }

    fn required_tools(&self) -> Vec<&str> {
        vec!["claude", "github"]
    }

    fn run(&self, ctx: &mut Context, tools: &LoggedToolRegistry) -> anyhow::Result<()> {
        let pr_number = ctx
            .artifacts
            .get("pr_number")
            .ok_or_else(|| anyhow::anyhow!("No PR number in context — run execute first"))?
            .clone();

        // Get the PR diff
        println!("[approve] Fetching PR #{} diff...", pr_number);
        let diff_output = std::process::Command::new("gh")
            .args(["pr", "diff", &pr_number, "--repo", &ctx.repo])
            .output()?;

        let diff = if diff_output.status.success() {
            String::from_utf8_lossy(&diff_output.stdout).to_string()
        } else {
            "(diff unavailable)".into()
        };

        // Use Claude to generate review
        let prompt = format!(
            r#"Review this pull request diff for the repository {}.

PR #{}: {}

Diff:
```
{}
```

Provide a code review with:
1. **Overall assessment** — approve, request changes, or comment
2. **Issues found** — bugs, security concerns, style problems
3. **Suggestions** — improvements, alternative approaches
4. **Verdict** — one of: APPROVE, REQUEST_CHANGES, COMMENT

Be concise and constructive. Output your review as markdown."#,
            ctx.repo,
            pr_number,
            ctx.title,
            truncate_diff(&diff, 8000)
        );

        println!("[approve] Generating review with Claude...");
        let review = tools.run_tool(
            "claude",
            "generate",
            &HashMap::from([("prompt".into(), prompt)]),
        )?;

        // Determine review action from Claude's output
        let review_action = if review.to_lowercase().contains("approve")
            && !review.to_lowercase().contains("request_changes")
        {
            "approve"
        } else if review.to_lowercase().contains("request_changes")
            || review.to_lowercase().contains("request changes")
        {
            "request-changes"
        } else {
            "comment"
        };

        // Post review via gh
        println!("[approve] Posting {} review...", review_action);
        let review_result = tools.run_tool(
            "github",
            "review_pr",
            &HashMap::from([
                ("repo".into(), ctx.repo.clone()),
                ("pr_number".into(), pr_number.clone()),
                ("review_action".into(), review_action.into()),
                ("body".into(), review.clone()),
            ]),
        )?;

        ctx.artifacts.insert("review".into(), review);
        ctx.artifacts
            .insert("review_action".into(), review_action.into());
        ctx.artifacts.insert("review_result".into(), review_result);

        println!("[approve] Review posted: {}", review_action);

        Ok(())
    }
}

fn truncate_diff(diff: &str, max: usize) -> String {
    if diff.len() <= max {
        diff.to_string()
    } else {
        format!("{}...\n(truncated)", &diff[..max])
    }
}
