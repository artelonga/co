use crate::context::{Context, Stage};
use crate::engine::LoggedToolRegistry;
use crate::module::Module;
use std::collections::HashMap;

/// Plan module: idea → GitHub issue
/// Uses Claude to generate structured issue content, then gh to create it.
pub struct PlanModule;

impl Module for PlanModule {
    fn name(&self) -> &str {
        "plan"
    }

    fn stage(&self) -> Stage {
        Stage::Plan
    }

    fn required_tools(&self) -> Vec<&str> {
        vec!["claude", "github"]
    }

    fn run(&self, ctx: &mut Context, tools: &LoggedToolRegistry) -> anyhow::Result<()> {
        // Gather knowledge context if obsidian is available
        let knowledge_context = if let Some(handle) = tools.get("obsidian")
            && handle.available()
        {
            let search_params = HashMap::from([("query".into(), ctx.title.clone())]);
            tools
                .run_tool("obsidian", "search", &search_params)
                .unwrap_or_default()
        } else {
            String::new()
        };

        // Generate issue content with Claude
        let prompt = format!(
            r#"You are creating a GitHub issue for the repository: {}

Title: {}
Description: {}

{}

Generate a well-structured GitHub issue body in Markdown with:
1. **Summary** — 2-3 sentences explaining the problem/feature
2. **Acceptance Criteria** — bulleted checklist of what "done" looks like
3. **Technical Notes** — implementation hints, relevant files, dependencies
4. **Labels suggestion** — comma-separated list of appropriate labels

Output ONLY the issue body markdown, no preamble."#,
            ctx.repo,
            ctx.title,
            ctx.description,
            if knowledge_context.is_empty() {
                String::new()
            } else {
                format!(
                    "Relevant context from knowledge base:\n{}",
                    knowledge_context
                )
            }
        );

        println!("[plan] Generating issue content with Claude...");
        let issue_body = tools.run_tool(
            "claude",
            "generate",
            &HashMap::from([("prompt".into(), prompt)]),
        )?;

        // Extract labels if suggested
        let labels = extract_labels(&issue_body);

        // Create the issue via gh
        println!("[plan] Creating issue on {}...", ctx.repo);
        let mut issue_params = HashMap::from([
            ("repo".into(), ctx.repo.clone()),
            ("title".into(), ctx.title.clone()),
            ("body".into(), issue_body.clone()),
        ]);

        if !labels.is_empty() {
            issue_params.insert("labels".into(), labels);
        }

        let issue_url = tools.run_tool("github", "create_issue", &issue_params)?;
        println!("[plan] Issue created: {}", issue_url);

        // Extract issue number from URL
        if let Some(number) = issue_url.rsplit('/').next() {
            ctx.artifacts
                .insert("issue_number".into(), number.to_string());
        }
        ctx.artifacts.insert("issue_url".into(), issue_url);
        ctx.artifacts.insert("issue_body".into(), issue_body);

        Ok(())
    }
}

/// Try to extract label suggestions from the generated body
fn extract_labels(body: &str) -> String {
    for line in body.lines() {
        let lower = line.to_lowercase();
        if lower.contains("label")
            && lower.contains(':')
            && let Some(after) = line.split(':').nth(1)
        {
            return after.trim().replace("**", "").replace('`', "");
        }
    }
    String::new()
}
