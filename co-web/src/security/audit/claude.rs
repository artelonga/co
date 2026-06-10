//! CO-388: `ClaudeSecurityBackend` — Claude AI-powered security scanner.
//!
//! Activates when `CO_SECURITY_BACKEND=claude` and `CO_SECURITY_API_KEY` is set.
//! Sends file diffs to the Claude API with a security audit system prompt and
//! parses structured JSON findings from the response.
//!
//! Cost guardrails (applied before any API call):
//!  - `CO_SECURITY_MAX_SCANS_PER_DAY` (default 50) — daily scan cap
//!  - Skip draft PRs, docs-only PRs, revert PRs (enforced by callers)

use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::{AuditResult, Category, Finding, PatchSuggestion, SecurityAuditBackend, Severity};

const CLAUDE_API_URL: &str = "https://api.anthropic.com/v1/messages";
const CLAUDE_MODEL: &str = "claude-sonnet-4-6";
const MAX_TOKENS: u32 = 4096;
const DEFAULT_MAX_SCANS_PER_DAY: u32 = 50;

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ClaudeRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<ClaudeMessage<'a>>,
}

#[derive(Serialize)]
struct ClaudeMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
}

#[derive(Deserialize)]
struct ClaudeContent {
    text: String,
}

/// Structured finding as returned by Claude.
#[derive(Deserialize)]
struct RawFinding {
    severity: String,
    category: String,
    file_path: String,
    line_start: usize,
    line_end: usize,
    description: String,
    cwe: Option<String>,
    cve_match: Option<String>,
    suggested_patch: Option<String>,
}

const SECURITY_SYSTEM_PROMPT: &str = "\
You are a security code reviewer. Analyse the provided source code diff for vulnerabilities. \
Focus on: SQL injection, XSS, authentication bypass, CSRF, path traversal, hardcoded secrets, \
insecure deserialisation, SSRF, and command injection. \
Return ONLY a JSON array of findings with this schema:\n\
[{\"severity\":\"critical|high|medium|low|info\",\"category\":\"sql_injection|xss|auth_bypass|csrf|path_traversal|hardcoded_secret|insecure_deserialization|ssrf|command_injection|other\",\
\"file_path\":\"string\",\"line_start\":1,\"line_end\":1,\"description\":\"string\",\
\"cwe\":\"CWE-N or null\",\"cve_match\":\"CVE-... or null\",\"suggested_patch\":\"string or null\"}]\n\
If no findings, return []. Do not include any explanation outside the JSON array.";

// ---------------------------------------------------------------------------
// ClaudeSecurityBackend
// ---------------------------------------------------------------------------

pub struct ClaudeSecurityBackend {
    api_key: String,
    max_scans_per_day: u32,
    scan_count: Arc<AtomicU32>,
    http: reqwest::Client,
}

impl ClaudeSecurityBackend {
    pub fn from_env() -> Self {
        let api_key = std::env::var("CO_SECURITY_API_KEY").unwrap_or_default();
        let max_scans = std::env::var("CO_SECURITY_MAX_SCANS_PER_DAY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_MAX_SCANS_PER_DAY);

        if api_key.is_empty() {
            warn!(
                "security: CO_SECURITY_API_KEY not set — ClaudeSecurityBackend will return empty findings"
            );
        }

        Self {
            api_key,
            max_scans_per_day: max_scans,
            scan_count: Arc::new(AtomicU32::new(0)),
            http: reqwest::Client::new(),
        }
    }

    fn check_rate_limit(&self) -> bool {
        let count = self.scan_count.fetch_add(1, Ordering::Relaxed);
        if count >= self.max_scans_per_day {
            warn!(
                "security: daily scan limit ({}) reached — skipping Claude scan",
                self.max_scans_per_day
            );
            return false;
        }
        true
    }

    async fn call_claude(&self, diff_content: &str) -> Result<Vec<Finding>, anyhow::Error> {
        if self.api_key.is_empty() {
            return Ok(vec![]);
        }

        let prompt = format!(
            "Analyse this code diff for security vulnerabilities:\n\n```diff\n{diff_content}\n```"
        );

        let req = ClaudeRequest {
            model: CLAUDE_MODEL,
            max_tokens: MAX_TOKENS,
            system: SECURITY_SYSTEM_PROMPT,
            messages: vec![ClaudeMessage {
                role: "user",
                content: &prompt,
            }],
        };

        let resp = self
            .http
            .post(CLAUDE_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&req)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Claude API returned {status}: {body}"));
        }

        let claude_resp: ClaudeResponse = resp.json().await?;
        let text = claude_resp
            .content
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join("");

        parse_findings(&text)
    }
}

fn parse_findings(text: &str) -> Result<Vec<Finding>, anyhow::Error> {
    // Extract the JSON array from the response (Claude may wrap it in backticks)
    let json_start = text.find('[').unwrap_or(0);
    let json_end = text.rfind(']').map(|i| i + 1).unwrap_or(text.len());
    let json_slice = &text[json_start..json_end];

    let raw: Vec<RawFinding> = serde_json::from_str(json_slice).map_err(|e| {
        anyhow::anyhow!("Failed to parse Claude findings JSON: {e}\nRaw: {json_slice}")
    })?;

    let findings = raw
        .into_iter()
        .map(|r| {
            let category = match r.category.as_str() {
                "sql_injection" => Category::SqlInjection,
                "xss" => Category::Xss,
                "auth_bypass" => Category::AuthBypass,
                "csrf" => Category::Csrf,
                "path_traversal" => Category::PathTraversal,
                "hardcoded_secret" => Category::HardcodedSecret,
                "insecure_deserialization" => Category::InsecureDeserialization,
                "ssrf" => Category::Ssrf,
                "command_injection" => Category::CommandInjection,
                other => Category::Other(other.to_string()),
            };
            let mut f = Finding::new(
                Severity::parse(&r.severity),
                category,
                r.file_path,
                (r.line_start, r.line_end),
                r.description,
            );
            f.cwe = r.cwe;
            f.cve_match = r.cve_match;
            if let Some(patch) = r.suggested_patch {
                f.suggested_patch = Some(PatchSuggestion {
                    description: patch,
                    diff: None,
                });
            }
            f
        })
        .collect();

    Ok(findings)
}

#[async_trait]
impl SecurityAuditBackend for ClaudeSecurityBackend {
    async fn scan_diff(&self, base_ref: &str, head_ref: &str) -> AuditResult {
        if !self.check_rate_limit() {
            return Ok(vec![]);
        }

        // Get the diff content via git
        let output = tokio::process::Command::new("git")
            .args(["diff", &format!("{base_ref}...{head_ref}")])
            .output()
            .await;

        let diff = match output {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).to_string(),
            Ok(out) => {
                warn!(
                    "security: git diff failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                return Ok(vec![]);
            }
            Err(e) => {
                warn!("security: git not available: {e}");
                return Ok(vec![]);
            }
        };

        if diff.is_empty() {
            info!("security: empty diff — no findings");
            return Ok(vec![]);
        }

        // Truncate diff to avoid token limits (~100KB)
        let diff_truncated = if diff.len() > 100_000 {
            &diff[..100_000]
        } else {
            &diff
        };

        self.call_claude(diff_truncated).await
    }

    async fn scan_full(&self, repo_path: &Path) -> AuditResult {
        if !self.check_rate_limit() {
            return Ok(vec![]);
        }

        // For full scans, collect the most security-critical files and scan them.
        // Priority order from the spec: vault writes, auth flows, WebSocket handlers,
        // markdown render paths.
        let priority_dirs = [
            repo_path.join("co-web/src/"),
            repo_path.join("co-web/static/"),
        ];

        let mut all_content = String::new();
        for dir in &priority_dirs {
            if !dir.exists() {
                continue;
            }
            let output = tokio::process::Command::new("find")
                .args([
                    dir.to_str().unwrap_or("."),
                    "-name",
                    "*.rs",
                    "-o",
                    "-name",
                    "*.ts",
                    "-o",
                    "-name",
                    "*.sql",
                ])
                .output()
                .await;

            if let Ok(out) = output {
                for path in String::from_utf8_lossy(&out.stdout).lines() {
                    if let Ok(content) = tokio::fs::read_to_string(path).await {
                        all_content.push_str(&format!("// FILE: {path}\n"));
                        all_content.push_str(&content);
                        all_content.push('\n');
                        if all_content.len() > 80_000 {
                            break;
                        }
                    }
                }
            }
        }

        if all_content.is_empty() {
            return Ok(vec![]);
        }

        self.call_claude(&all_content).await
    }

    async fn suggest_patch(
        &self,
        finding: &Finding,
    ) -> Result<Option<PatchSuggestion>, anyhow::Error> {
        if self.api_key.is_empty() {
            return Ok(None);
        }

        let prompt = format!(
            "Suggest a minimal, safe code fix for this security finding:\n\
             Severity: {}\nCategory: {}\nFile: {}\nDescription: {}\n\
             Respond with only the fix description (1-2 sentences).",
            finding.severity.as_str(),
            finding.category.as_str(),
            finding.file_path,
            finding.description,
        );

        let req = ClaudeRequest {
            model: CLAUDE_MODEL,
            max_tokens: 256,
            system: "You are a security expert suggesting minimal code fixes.",
            messages: vec![ClaudeMessage {
                role: "user",
                content: &prompt,
            }],
        };

        let resp = self
            .http
            .post(CLAUDE_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&req)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(None);
        }

        let claude_resp: ClaudeResponse = resp.json().await?;
        let text = claude_resp
            .content
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join("");

        Ok(Some(PatchSuggestion {
            description: text.trim().to_string(),
            diff: None,
        }))
    }

    fn name(&self) -> &'static str {
        "claude-security"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_findings_empty_array() {
        let findings = parse_findings("[]").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn parse_findings_single_xss() {
        let json = r#"[{"severity":"high","category":"xss","file_path":"app.ts","line_start":10,"line_end":10,"description":"innerHTML","cwe":"CWE-79","cve_match":null,"suggested_patch":null}]"#;
        let findings = parse_findings(json).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::High);
    }

    #[test]
    fn parse_findings_with_backtick_wrapper() {
        let text = "```json\n[{\"severity\":\"info\",\"category\":\"other\",\"file_path\":\"f.rs\",\"line_start\":1,\"line_end\":1,\"description\":\"ok\",\"cwe\":null,\"cve_match\":null,\"suggested_patch\":null}]\n```";
        let findings = parse_findings(text).unwrap();
        assert_eq!(findings.len(), 1);
    }
}
