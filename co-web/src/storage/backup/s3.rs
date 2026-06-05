//! CO-365: S3 backup backend stub.
//! Real implementation deferred to v3.1+; compile-tested via `backup-s3` feature.

use std::path::Path;

use async_trait::async_trait;

use super::{BackupBackend, Snapshot, SnapshotId, SnapshotMeta};

pub struct S3Backend;

impl S3Backend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for S3Backend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BackupBackend for S3Backend {
    async fn put(&self, _snapshot: &Snapshot) -> anyhow::Result<SnapshotId> {
        unimplemented!("S3 backend not yet implemented; enable backup-s3 feature in v3.1+")
    }

    async fn list(&self) -> anyhow::Result<Vec<SnapshotMeta>> {
        unimplemented!("S3 backend not yet implemented")
    }

    async fn fetch(&self, _id: &SnapshotId, _dest: &Path) -> anyhow::Result<()> {
        unimplemented!("S3 backend not yet implemented")
    }

    async fn delete(&self, _id: &SnapshotId) -> anyhow::Result<()> {
        unimplemented!("S3 backend not yet implemented")
    }

    fn name(&self) -> &'static str {
        "s3"
    }
}
