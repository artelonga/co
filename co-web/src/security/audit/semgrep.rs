//! CO-388: `SemgrepBackend` — stub (feature `security-semgrep`).
//!
//! Fill this in when Semgrep integration is needed.
//! Enable with: `cargo build -p co-web --features security-semgrep`

use std::path::Path;

use async_trait::async_trait;

use super::{AuditResult, Finding, PatchSuggestion, SecurityAuditBackend};

pub struct SemgrepBackend;

#[async_trait]
impl SecurityAuditBackend for SemgrepBackend {
    async fn scan_diff(&self, _base_ref: &str, _head_ref: &str) -> AuditResult {
        Ok(vec![])
    }

    async fn scan_full(&self, _repo_path: &Path) -> AuditResult {
        Ok(vec![])
    }

    async fn suggest_patch(
        &self,
        _finding: &Finding,
    ) -> Result<Option<PatchSuggestion>, anyhow::Error> {
        Ok(None)
    }

    fn name(&self) -> &'static str {
        "semgrep"
    }
}
