//! CO-365: Fly.io volume snapshot backend stub.
//! Uses `flyctl volumes snapshots` shell call.
//! Real implementation deferred to v3.1+; compile-tested via `backup-fly` feature.

use std::path::Path;

use async_trait::async_trait;

use super::{BackupBackend, Snapshot, SnapshotId, SnapshotMeta};

pub struct FlyVolumeBackend;

impl FlyVolumeBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FlyVolumeBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BackupBackend for FlyVolumeBackend {
    async fn put(&self, _snapshot: &Snapshot) -> anyhow::Result<SnapshotId> {
        unimplemented!("Fly volume backend not yet implemented; enable backup-fly feature in v3.1+")
    }

    async fn list(&self) -> anyhow::Result<Vec<SnapshotMeta>> {
        unimplemented!("Fly volume backend not yet implemented")
    }

    async fn fetch(&self, _id: &SnapshotId, _dest: &Path) -> anyhow::Result<()> {
        unimplemented!("Fly volume backend not yet implemented")
    }

    async fn delete(&self, _id: &SnapshotId) -> anyhow::Result<()> {
        unimplemented!("Fly volume backend not yet implemented")
    }

    fn name(&self) -> &'static str {
        "fly"
    }
}
