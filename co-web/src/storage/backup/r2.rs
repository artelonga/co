//! CO-365: Cloudflare R2 backup backend stub.
//! Real implementation deferred to v3.1+; compile-tested via `backup-r2` feature.

use std::path::Path;

use async_trait::async_trait;

use super::{BackupBackend, Snapshot, SnapshotId, SnapshotMeta};

pub struct R2Backend;

impl R2Backend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for R2Backend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BackupBackend for R2Backend {
    async fn put(&self, _snapshot: &Snapshot) -> anyhow::Result<SnapshotId> {
        unimplemented!("R2 backend not yet implemented; enable backup-r2 feature in v3.1+")
    }

    async fn list(&self) -> anyhow::Result<Vec<SnapshotMeta>> {
        unimplemented!("R2 backend not yet implemented")
    }

    async fn fetch(&self, _id: &SnapshotId, _dest: &Path) -> anyhow::Result<()> {
        unimplemented!("R2 backend not yet implemented")
    }

    async fn delete(&self, _id: &SnapshotId) -> anyhow::Result<()> {
        unimplemented!("R2 backend not yet implemented")
    }

    fn name(&self) -> &'static str {
        "r2"
    }
}
