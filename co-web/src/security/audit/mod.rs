//! CO-388: `SecurityAuditBackend` trait + core types.
//!
//! Implementations:
//!  - `LocalGrepBackend` — default, pattern-based (always available)
//!  - `ClaudeSecurityBackend` — full, activates when CO_SECURITY_BACKEND=claude + API key
//!  - `NoOpBackend`           — dev-only, CO_SECURITY_BACKEND=disabled
//!  - `SemgrepBackend`        — stub, feature-gated `security-semgrep`
//!  - `SonarBackend`          — stub, feature-gated `security-sonar`

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::eda::event::new_ulid;

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Critical => "critical",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "critical" => Severity::Critical,
            "high" => Severity::High,
            "medium" => Severity::Medium,
            "low" => Severity::Low,
            _ => Severity::Info,
        }
    }

    pub fn blocks_merge(&self) -> bool {
        matches!(self, Severity::Critical | Severity::High)
    }

    pub fn creates_pbi(&self) -> bool {
        matches!(self, Severity::Critical | Severity::High | Severity::Medium)
    }
}

// ---------------------------------------------------------------------------
// Category
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    SqlInjection,
    Xss,
    AuthBypass,
    Csrf,
    PathTraversal,
    HardcodedSecret,
    InsecureDeserialization,
    Ssrf,
    CommandInjection,
    Other(String),
}

impl Category {
    pub fn as_str(&self) -> &str {
        match self {
            Category::SqlInjection => "sql_injection",
            Category::Xss => "xss",
            Category::AuthBypass => "auth_bypass",
            Category::Csrf => "csrf",
            Category::PathTraversal => "path_traversal",
            Category::HardcodedSecret => "hardcoded_secret",
            Category::InsecureDeserialization => "insecure_deserialization",
            Category::Ssrf => "ssrf",
            Category::CommandInjection => "command_injection",
            Category::Other(s) => s.as_str(),
        }
    }
}

// ---------------------------------------------------------------------------
// PatchSuggestion
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchSuggestion {
    pub description: String,
    pub diff: Option<String>,
}

// ---------------------------------------------------------------------------
// Finding
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// ULID — chronologically sortable unique identifier.
    pub id: String,
    pub severity: Severity,
    pub category: Category,
    pub file_path: String,
    pub line_range: (usize, usize),
    pub description: String,
    pub cwe: Option<String>,
    pub cve_match: Option<String>,
    pub suggested_patch: Option<PatchSuggestion>,
}

impl Finding {
    pub fn new(
        severity: Severity,
        category: Category,
        file_path: impl Into<String>,
        line_range: (usize, usize),
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: new_ulid(),
            severity,
            category,
            file_path: file_path.into(),
            line_range,
            description: description.into(),
            cwe: None,
            cve_match: None,
            suggested_patch: None,
        }
    }

    pub fn with_cwe(mut self, cwe: impl Into<String>) -> Self {
        self.cwe = Some(cwe.into());
        self
    }
}

pub type AuditResult = Result<Vec<Finding>, anyhow::Error>;

// ---------------------------------------------------------------------------
// SecurityAuditBackend trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait SecurityAuditBackend: Send + Sync {
    /// Scan a codebase diff and return findings.
    async fn scan_diff(&self, base_ref: &str, head_ref: &str) -> AuditResult;

    /// Scan an entire codebase snapshot rooted at `repo_path`.
    async fn scan_full(&self, repo_path: &Path) -> AuditResult;

    /// Suggest a patch for a given finding.
    async fn suggest_patch(
        &self,
        finding: &Finding,
    ) -> Result<Option<PatchSuggestion>, anyhow::Error>;

    /// Human-readable backend name for logging/metrics.
    fn name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Sub-modules
// ---------------------------------------------------------------------------

pub mod claude;
pub mod local_grep;
pub mod noop;

#[cfg(feature = "security-semgrep")]
pub mod semgrep;
#[cfg(feature = "security-sonar")]
pub mod sonar;

pub use claude::ClaudeSecurityBackend;
pub use local_grep::LocalGrepBackend;
pub use noop::NoOpBackend;

// ---------------------------------------------------------------------------
// Backend factory
// ---------------------------------------------------------------------------

/// Build the active security audit backend from environment variables.
///
/// `CO_SECURITY_BACKEND` controls selection:
/// - `"claude"`    — ClaudeSecurityBackend (requires CO_SECURITY_API_KEY)
/// - `"disabled"`  — NoOpBackend (dev/test only)
/// - unset or other — LocalGrepBackend (default)
pub fn build_backend() -> Arc<dyn SecurityAuditBackend> {
    match std::env::var("CO_SECURITY_BACKEND")
        .as_deref()
        .unwrap_or("")
    {
        "claude" => Arc::new(ClaudeSecurityBackend::from_env()),
        "disabled" => Arc::new(NoOpBackend),
        _ => Arc::new(LocalGrepBackend::new()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ordering() {
        assert!(Severity::Info < Severity::Low);
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::Medium < Severity::High);
        assert!(Severity::High < Severity::Critical);
    }

    #[test]
    fn severity_blocks_merge_only_for_high_critical() {
        assert!(!Severity::Info.blocks_merge());
        assert!(!Severity::Low.blocks_merge());
        assert!(!Severity::Medium.blocks_merge());
        assert!(Severity::High.blocks_merge());
        assert!(Severity::Critical.blocks_merge());
    }

    #[test]
    fn severity_creates_pbi_for_medium_plus() {
        assert!(!Severity::Info.creates_pbi());
        assert!(!Severity::Low.creates_pbi());
        assert!(Severity::Medium.creates_pbi());
        assert!(Severity::High.creates_pbi());
        assert!(Severity::Critical.creates_pbi());
    }

    #[test]
    fn finding_new_assigns_ulid() {
        let f = Finding::new(
            Severity::High,
            Category::Xss,
            "co-web/static/main.ts",
            (42, 42),
            "Unescaped innerHTML assignment",
        );
        assert_eq!(f.id.len(), 26);
        assert_eq!(f.severity, Severity::High);
    }

    #[test]
    fn severity_round_trips() {
        for s in &["info", "low", "medium", "high", "critical"] {
            let sev = Severity::parse(s);
            assert_eq!(sev.as_str(), *s);
        }
    }
}
