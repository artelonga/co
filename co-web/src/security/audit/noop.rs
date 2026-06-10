//! CO-388: `NoOpBackend` — dev/test backend that returns zero findings.
//!
//! Activated via `CO_SECURITY_BACKEND=disabled`. Never use in production.

use std::path::Path;

use async_trait::async_trait;

use super::{AuditResult, Finding, PatchSuggestion, SecurityAuditBackend};

pub struct NoOpBackend;

#[async_trait]
impl SecurityAuditBackend for NoOpBackend {
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
        "noop"
    }
}
