use crate::tool::Tool;
use std::collections::HashMap;
use std::process::Command;

/// Wraps the `gh` CLI for issue/PR operations on arbitrary repos.
pub struct GitHubTool;

impl Tool for GitHubTool {
    fn name(&self) -> &str {
        "github"
    }

    fn description(&self) -> &str {
        "GitHub CLI (gh) — create issues, PRs, reviews on any repo"
    }

    fn available(&self) -> bool {
        Command::new("gh")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run(&self, action: &str, params: &HashMap<String, String>) -> anyhow::Result<String> {
        match action {
            "create_issue" => self.create_issue(params),
            "create_pr" => self.create_pr(params),
            "review_pr" => self.review_pr(params),
            "get_issue" => self.get_issue(params),
            "list_issues" => self.list_issues(params),
            _ => anyhow::bail!("Unknown github action: {}", action),
        }
    }
}

impl GitHubTool {
    fn create_issue(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let repo = params
            .get("repo")
            .ok_or_else(|| anyhow::anyhow!("Missing 'repo' param"))?;
        let title = params
            .get("title")
            .ok_or_else(|| anyhow::anyhow!("Missing 'title' param"))?;
        let body = params.get("body").map(|s| s.as_str()).unwrap_or("");

        let mut cmd = Command::new("gh");
        cmd.args([
            "issue", "create", "--repo", repo, "--title", title, "--body", body,
        ]);

        if let Some(labels) = params.get("labels") {
            cmd.args(["--label", labels]);
        }

        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh issue create failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn create_pr(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let repo = params
            .get("repo")
            .ok_or_else(|| anyhow::anyhow!("Missing 'repo' param"))?;
        let title = params
            .get("title")
            .ok_or_else(|| anyhow::anyhow!("Missing 'title' param"))?;
        let body = params.get("body").map(|s| s.as_str()).unwrap_or("");
        let head = params
            .get("head")
            .ok_or_else(|| anyhow::anyhow!("Missing 'head' (branch) param"))?;

        let mut cmd = Command::new("gh");
        cmd.args([
            "pr", "create", "--repo", repo, "--title", title, "--body", body, "--head", head,
        ]);

        if let Some(base) = params.get("base") {
            cmd.args(["--base", base]);
        }

        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh pr create failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn review_pr(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let repo = params
            .get("repo")
            .ok_or_else(|| anyhow::anyhow!("Missing 'repo' param"))?;
        let pr_number = params
            .get("pr_number")
            .ok_or_else(|| anyhow::anyhow!("Missing 'pr_number' param"))?;
        let action = params
            .get("review_action")
            .map(|s| s.as_str())
            .unwrap_or("comment");
        let body = params.get("body").map(|s| s.as_str()).unwrap_or("");

        let output = Command::new("gh")
            .args([
                "pr",
                "review",
                pr_number,
                "--repo",
                repo,
                &format!("--{}", action),
                "--body",
                body,
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh pr review failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn get_issue(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let repo = params
            .get("repo")
            .ok_or_else(|| anyhow::anyhow!("Missing 'repo' param"))?;
        let number = params
            .get("number")
            .ok_or_else(|| anyhow::anyhow!("Missing 'number' param"))?;

        let output = Command::new("gh")
            .args([
                "issue",
                "view",
                number,
                "--repo",
                repo,
                "--json",
                "title,body,labels,state",
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh issue view failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn list_issues(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let repo = params
            .get("repo")
            .ok_or_else(|| anyhow::anyhow!("Missing 'repo' param"))?;

        let mut cmd = Command::new("gh");
        cmd.args([
            "issue",
            "list",
            "--repo",
            repo,
            "--json",
            "number,title,labels,state",
        ]);

        if let Some(limit) = params.get("limit") {
            cmd.args(["--limit", limit]);
        }

        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("gh issue list failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}
