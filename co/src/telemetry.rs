use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A single tool invocation record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    pub action: String,
    pub params: HashMap<String, String>,
    pub output: String,
    pub success: bool,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
}

/// A pipeline stage execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageRecord {
    pub stage: String,
    pub module: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub success: bool,
    pub error: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub artifacts_produced: Vec<String>,
}

/// Complete run log with telemetry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLog {
    pub run_id: String,
    pub repo: String,
    pub title: String,
    pub description: String,
    pub user: UserInfo,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub stages: Vec<StageRecord>,
    pub artifacts: HashMap<String, String>,
    pub summary: Option<String>,
}

/// User information captured at run time
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub username: String,
    pub hostname: String,
    pub working_dir: String,
    /// Active git identity name (from IdentityStore)
    pub git_identity: String,
    /// GitHub username associated with the active identity
    pub github_user: String,
    /// Git email for the active identity
    pub git_email: String,
}

impl UserInfo {
    pub fn capture() -> Self {
        let identity_store = crate::identity::IdentityStore::load();
        let active = identity_store.active_identity();

        Self {
            username: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "unknown".into()),
            hostname: hostname(),
            working_dir: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".into()),
            git_identity: identity_store.active.clone(),
            github_user: active
                .map(|id| id.github_user.clone())
                .unwrap_or_else(|| "unknown".into()),
            git_email: active
                .map(|id| id.git_email.clone())
                .unwrap_or_else(|| "unknown".into()),
        }
    }
}

impl RunLog {
    pub fn new(run_id: &str, repo: &str, title: &str, description: &str) -> Self {
        Self {
            run_id: run_id.into(),
            repo: repo.into(),
            title: title.into(),
            description: description.into(),
            user: UserInfo::capture(),
            started_at: Utc::now(),
            finished_at: None,
            duration_ms: None,
            stages: Vec::new(),
            artifacts: HashMap::new(),
            summary: None,
        }
    }

    pub fn start_stage(&mut self, stage: &str, module: &str) -> usize {
        let record = StageRecord {
            stage: stage.into(),
            module: module.into(),
            started_at: Utc::now(),
            finished_at: None,
            duration_ms: None,
            success: false,
            error: None,
            tool_calls: Vec::new(),
            artifacts_produced: Vec::new(),
        };
        self.stages.push(record);
        self.stages.len() - 1
    }

    pub fn finish_stage(&mut self, idx: usize, success: bool, error: Option<String>) {
        if let Some(stage) = self.stages.get_mut(idx) {
            let now = Utc::now();
            stage.finished_at = Some(now);
            stage.duration_ms = Some((now - stage.started_at).num_milliseconds() as u64);
            stage.success = success;
            stage.error = error;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_tool_call(
        &mut self,
        stage_idx: usize,
        tool: &str,
        action: &str,
        params: &HashMap<String, String>,
        output: &str,
        success: bool,
        duration_ms: u64,
    ) {
        if let Some(stage) = self.stages.get_mut(stage_idx) {
            stage.tool_calls.push(ToolCall {
                tool: tool.into(),
                action: action.into(),
                params: sanitize_params(params),
                output: truncate(output, 4000),
                success,
                duration_ms,
                timestamp: Utc::now(),
            });
        }
    }

    pub fn finalize(&mut self, artifacts: &HashMap<String, String>, summary: Option<String>) {
        let now = Utc::now();
        self.finished_at = Some(now);
        self.duration_ms = Some((now - self.started_at).num_milliseconds() as u64);
        self.artifacts = artifacts.clone();
        self.summary = summary;
    }

    /// Save the run log to disk
    pub fn save(&self, logs_dir: &Path) -> anyhow::Result<PathBuf> {
        std::fs::create_dir_all(logs_dir)?;
        let filename = format!(
            "{}_{}.yaml",
            self.started_at.format("%Y%m%d_%H%M%S"),
            &self.run_id[..8]
        );
        let path = logs_dir.join(&filename);
        let yaml = serde_yaml::to_string(self)?;
        std::fs::write(&path, yaml)?;
        Ok(path)
    }

    /// Generate a human-readable summary
    pub fn render_summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("# Run Report: {}", self.title));
        lines.push(format!("Run ID: {}", self.run_id));
        lines.push(format!("Repository: {}", self.repo));
        lines.push(format!(
            "User: {}@{} (identity: {}, gh: {})",
            self.user.username, self.user.hostname, self.user.git_identity, self.user.github_user
        ));
        lines.push(format!(
            "Started: {}",
            self.started_at.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        if let Some(dur) = self.duration_ms {
            lines.push(format!("Duration: {:.1}s", dur as f64 / 1000.0));
        }
        lines.push(String::new());

        lines.push("## Stages".into());
        for stage in &self.stages {
            let status = if stage.success { "OK" } else { "FAILED" };
            let dur = stage
                .duration_ms
                .map(|d| format!("{:.1}s", d as f64 / 1000.0))
                .unwrap_or_else(|| "?".into());
            lines.push(format!(
                "  {} ({}) — {} [{}]",
                stage.stage, stage.module, status, dur
            ));

            for call in &stage.tool_calls {
                let call_status = if call.success { "ok" } else { "err" };
                lines.push(format!(
                    "    {} {} — {} ({}ms)",
                    call.tool, call.action, call_status, call.duration_ms
                ));
            }

            if let Some(ref err) = stage.error {
                lines.push(format!("    Error: {}", err));
            }
        }

        if !self.artifacts.is_empty() {
            lines.push(String::new());
            lines.push("## Artifacts".into());
            for (key, value) in &self.artifacts {
                let display = if value.len() > 120 {
                    format!("{}...", &value[..120])
                } else {
                    value.clone()
                };
                lines.push(format!("  {}: {}", key, display));
            }
        }

        if let Some(ref summary) = self.summary {
            lines.push(String::new());
            lines.push("## Summary".into());
            lines.push(summary.clone());
        }

        lines.join("\n")
    }
}

/// Redact sensitive params (tokens, passwords)
fn sanitize_params(params: &HashMap<String, String>) -> HashMap<String, String> {
    params
        .iter()
        .map(|(k, v)| {
            let sanitized = if k.to_lowercase().contains("token")
                || k.to_lowercase().contains("password")
                || k.to_lowercase().contains("secret")
            {
                "***REDACTED***".into()
            } else {
                v.clone()
            };
            (k.clone(), sanitized)
        })
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...(truncated)", &s[..max])
    }
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}
