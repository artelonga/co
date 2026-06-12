//! CO-388: `SonarBackend` — stub (feature `security-sonar`).
//!
//! Fill this in when SonarQube integration is needed.
//! Enable with: `cargo build -p co-web --features security-sonar`

use std::path::Path;

use async_trait::async_trait;

use super::{AuditResult, Finding, PatchSuggestion, SecurityAuditBackend};

pub struct SonarBackend;

#[async_trait]
impl SecurityAuditBackend for SonarBackend {
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
        "sonar"
    }
}
