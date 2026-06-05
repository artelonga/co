//! CO-365: Local filesystem backup backend.
//!
//! Stores snapshots under `$CO_BACKUP_DIR` (default `/data/backups/`).
//! Each snapshot is a `.tar.gz` file plus a sidecar `.json` with metadata.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{BackupBackend, Snapshot, SnapshotId, SnapshotMeta};

/// Sidecar file written alongside each `.tar.gz` to avoid re-hashing on list().
#[derive(Debug, Serialize, Deserialize)]
struct Sidecar {
    id: String,
    created_at: DateTime<Utc>,
    bytes: u64,
    sha256: String,
}

pub struct LocalFsBackend {
    dir: PathBuf,
}

impl LocalFsBackend {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn sidecar_path(&self, id: &SnapshotId) -> PathBuf {
        self.dir.join(format!("{}.json", id.0))
    }

    fn tarball_path(&self, id: &SnapshotId) -> PathBuf {
        self.dir.join(&id.0)
    }
}

#[async_trait]
impl BackupBackend for LocalFsBackend {
    async fn put(&self, snapshot: &Snapshot) -> anyhow::Result<SnapshotId> {
        std::fs::create_dir_all(&self.dir)?;

        // Derive the snapshot ID from the tarball filename
        let filename = snapshot
            .tarball_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("snapshot tarball has no filename"))?;
        let id = SnapshotId(filename.to_string());

        let dest = self.tarball_path(&id);
        if dest != snapshot.tarball_path {
            std::fs::copy(&snapshot.tarball_path, &dest)?;
        }

        let sidecar = Sidecar {
            id: id.0.clone(),
            created_at: snapshot.created_at,
            bytes: snapshot.bytes,
            sha256: snapshot.sha256.clone(),
        };
        let json = serde_json::to_string_pretty(&sidecar)?;
        std::fs::write(self.sidecar_path(&id), json)?;

        Ok(id)
    }

    async fn list(&self) -> anyhow::Result<Vec<SnapshotMeta>> {
        if !self.dir.exists() {
            return Ok(vec![]);
        }

        let mut metas: Vec<SnapshotMeta> = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".tar.gz") {
                continue;
            }
            let id = SnapshotId(name.clone());
            let sidecar_path = self.sidecar_path(&id);
            if sidecar_path.exists()
                && let Ok(json) = std::fs::read_to_string(&sidecar_path)
                && let Ok(s) = serde_json::from_str::<Sidecar>(&json)
            {
                metas.push(SnapshotMeta {
                    id,
                    created_at: s.created_at,
                    bytes: s.bytes,
                    sha256: s.sha256,
                });
                continue;
            }
            // No sidecar — fill in what we can from the filename
            let meta = entry.metadata()?;
            metas.push(SnapshotMeta {
                id,
                created_at: Utc::now(),
                bytes: meta.len(),
                sha256: String::new(),
            });
        }

        // Newest first
        metas.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(metas)
    }

    async fn fetch(&self, id: &SnapshotId, dest: &Path) -> anyhow::Result<()> {
        let src = self.tarball_path(id);
        if !src.exists() {
            anyhow::bail!("snapshot {} not found in local backend", id);
        }
        std::fs::copy(&src, dest)?;
        Ok(())
    }

    async fn delete(&self, id: &SnapshotId) -> anyhow::Result<()> {
        let tarball = self.tarball_path(id);
        if tarball.exists() {
            std::fs::remove_file(&tarball)?;
        }
        let sidecar = self.sidecar_path(id);
        if sidecar.exists() {
            std::fs::remove_file(&sidecar)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "local"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_snapshot(dir: &Path, filename: &str, content: &[u8]) -> Snapshot {
        let tarball_path = dir.join(filename);
        std::fs::write(&tarball_path, content).unwrap();
        Snapshot {
            created_at: Utc::now(),
            manifest_json: serde_json::json!({}),
            tarball_path,
            bytes: content.len() as u64,
            sha256: "deadbeef".to_string(),
        }
    }

    #[tokio::test]
    async fn put_stores_tarball_and_sidecar() {
        let tmp = tempdir().unwrap();
        let backend = LocalFsBackend::new(tmp.path().join("backups"));
        let snapshot = make_snapshot(
            tmp.path(),
            "snapshot-20260101T000000Z-abcd1234.tar.gz",
            b"fake-tarball",
        );

        let id = backend.put(&snapshot).await.unwrap();
        assert_eq!(id.0, "snapshot-20260101T000000Z-abcd1234.tar.gz");
        assert!(backend.tarball_path(&id).exists());
        assert!(backend.sidecar_path(&id).exists());
    }

    #[tokio::test]
    async fn list_returns_snapshots_newest_first() {
        let tmp = tempdir().unwrap();
        let backend = LocalFsBackend::new(tmp.path().join("backups"));

        let s1 = make_snapshot(
            tmp.path(),
            "snapshot-20260101T000000Z-aaaa0000.tar.gz",
            b"a",
        );
        let s2 = make_snapshot(
            tmp.path(),
            "snapshot-20260102T000000Z-bbbb1111.tar.gz",
            b"b",
        );

        // Put s1 first, s2 second — list should return s2, s1 (newest first).
        // Fake different timestamps for ordering test.
        let mut s1_owned = s1;
        s1_owned.created_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut s2_owned = s2;
        s2_owned.created_at = chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        backend.put(&s1_owned).await.unwrap();
        backend.put(&s2_owned).await.unwrap();

        let list = backend.list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(
            list[0].created_at >= list[1].created_at,
            "should be newest first"
        );
    }

    #[tokio::test]
    async fn delete_removes_tarball_and_sidecar() {
        let tmp = tempdir().unwrap();
        let backend = LocalFsBackend::new(tmp.path().join("backups"));
        let snapshot = make_snapshot(
            tmp.path(),
            "snapshot-20260101T000000Z-cccc2222.tar.gz",
            b"c",
        );

        let id = backend.put(&snapshot).await.unwrap();
        assert!(backend.tarball_path(&id).exists());

        backend.delete(&id).await.unwrap();
        assert!(!backend.tarball_path(&id).exists());
        assert!(!backend.sidecar_path(&id).exists());
    }

    #[tokio::test]
    async fn fetch_copies_to_dest() {
        let tmp = tempdir().unwrap();
        let backend = LocalFsBackend::new(tmp.path().join("backups"));
        let snapshot = make_snapshot(
            tmp.path(),
            "snapshot-20260101T000000Z-dddd3333.tar.gz",
            b"fetch-content",
        );

        let id = backend.put(&snapshot).await.unwrap();
        let dest = tmp.path().join("fetched.tar.gz");
        backend.fetch(&id, &dest).await.unwrap();

        let content = std::fs::read(&dest).unwrap();
        assert_eq!(content, b"fetch-content");
    }

    #[tokio::test]
    async fn list_empty_dir_returns_empty() {
        let tmp = tempdir().unwrap();
        let backend = LocalFsBackend::new(tmp.path().join("backups"));
        let list = backend.list().await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn fetch_missing_snapshot_returns_error() {
        let tmp = tempdir().unwrap();
        let backend = LocalFsBackend::new(tmp.path().join("backups"));
        let id = SnapshotId("snapshot-missing.tar.gz".to_string());
        let result = backend.fetch(&id, Path::new("/tmp/nowhere")).await;
        assert!(result.is_err());
    }
}
