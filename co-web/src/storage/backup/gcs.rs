//! CO-365: Google Cloud Storage backup backend stub.
//! Real implementation deferred to v3.1+; compile-tested via `backup-gcs` feature.

use std::path::Path;

use async_trait::async_trait;

use super::{BackupBackend, Snapshot, SnapshotId, SnapshotMeta};

pub struct GcsBackend;

impl GcsBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GcsBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BackupBackend for GcsBackend {
    async fn put(&self, _snapshot: &Snapshot) -> anyhow::Result<SnapshotId> {
        unimplemented!("GCS backend not yet implemented; enable backup-gcs feature in v3.1+")
    }

    async fn list(&self) -> anyhow::Result<Vec<SnapshotMeta>> {
        unimplemented!("GCS backend not yet implemented")
    }

    async fn fetch(&self, _id: &SnapshotId, _dest: &Path) -> anyhow::Result<()> {
        unimplemented!("GCS backend not yet implemented")
    }

    async fn delete(&self, _id: &SnapshotId) -> anyhow::Result<()> {
        unimplemented!("GCS backend not yet implemented")
    }

    fn name(&self) -> &'static str {
        "gcs"
    }
}
