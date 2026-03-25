use crate::tool::Tool;
use std::collections::HashMap;
use std::process::Command;

/// Wraps the Claude Code CLI for LLM-assisted generation.
pub struct ClaudeTool {
    model: String,
}

impl ClaudeTool {
    pub fn new(model: String) -> Self {
        Self { model }
    }
}

impl Tool for ClaudeTool {
    fn name(&self) -> &str {
        "claude"
    }

    fn description(&self) -> &str {
        "Claude Code CLI — LLM-assisted code generation and analysis"
    }

    fn available(&self) -> bool {
        Command::new("claude")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn run(&self, action: &str, params: &HashMap<String, String>) -> anyhow::Result<String> {
        match action {
            "generate" => self.generate(params),
            "execute" => self.execute(params),
            _ => anyhow::bail!("Unknown claude action: {}", action),
        }
    }
}

impl ClaudeTool {
    /// Non-interactive generation: claude --print -p "prompt"
    /// Returns generated text without executing any tools.
    fn generate(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let prompt = params
            .get("prompt")
            .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' param"))?;

        let mut cmd = Command::new("claude");
        cmd.args(["--print", "-p", prompt, "--model", &self.model]);

        if let Some(dir) = params.get("workdir") {
            cmd.current_dir(dir);
        }

        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("claude generate failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Interactive execution: claude -p "prompt"
    /// Claude Code will use its tools (read/write files, run commands) to implement changes.
    fn execute(&self, params: &HashMap<String, String>) -> anyhow::Result<String> {
        let prompt = params
            .get("prompt")
            .ok_or_else(|| anyhow::anyhow!("Missing 'prompt' param"))?;

        let mut cmd = Command::new("claude");
        cmd.args(["-p", prompt, "--model", &self.model]);

        if let Some(dir) = params.get("workdir") {
            cmd.current_dir(dir);
        }

        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("claude execute failed: {}", stderr);
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}
