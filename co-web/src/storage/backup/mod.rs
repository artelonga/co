//! CO-365: Pluggable backup backend trait + snapshot types.
//!
//! `BackupBackend` abstracts over where snapshots are stored (local FS, S3,
//! R2, Fly volume, GCS).  The default `LocalFsBackend` is always available;
//! cloud backends compile only when the matching feature flag is enabled.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub mod local_fs;
pub mod snapshot_dir;
pub mod worker;

#[cfg(feature = "backup-s3")]
pub mod s3;

#[cfg(feature = "backup-r2")]
pub mod r2;

#[cfg(feature = "backup-fly")]
pub mod fly;

#[cfg(feature = "backup-gcs")]
pub mod gcs;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Opaque backend-specific identifier for a stored snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnapshotId(pub String);

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// An in-flight snapshot ready to be handed to `BackupBackend::put`.
///
/// The tarball at `tarball_path` is created by the backup worker; `put()`
/// moves or copies it to the backend's permanent store.
pub struct Snapshot {
    pub created_at: DateTime<Utc>,
    /// Full manifest as JSON (written inside the tarball as `manifest.json`).
    pub manifest_json: serde_json::Value,
    /// Local path of the `.tar.gz` created by the snapshot builder.
    pub tarball_path: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}

/// Lightweight metadata returned by `BackupBackend::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMeta {
    pub id: SnapshotId,
    pub created_at: DateTime<Utc>,
    pub bytes: u64,
    pub sha256: String,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait BackupBackend: Send + Sync {
    /// Store a snapshot; returns a backend-specific identifier.
    async fn put(&self, snapshot: &Snapshot) -> anyhow::Result<SnapshotId>;

    /// List stored snapshots, newest first.
    async fn list(&self) -> anyhow::Result<Vec<SnapshotMeta>>;

    /// Copy a stored snapshot to `dest` on the local filesystem.
    async fn fetch(&self, id: &SnapshotId, dest: &Path) -> anyhow::Result<()>;

    /// Delete a stored snapshot (used by the retention policy).
    async fn delete(&self, id: &SnapshotId) -> anyhow::Result<()>;

    /// Short name used in telemetry and manifest metadata.
    fn name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Factory: build a backend from CO_BACKUP_BACKEND env var
// ---------------------------------------------------------------------------

/// Returns `None` when `CO_BACKUP_BACKEND=disabled`.
/// Returns `Some(Box<dyn BackupBackend>)` for every other value (default: `local`).
pub fn backend_from_env(data_dir: &Path) -> Option<Box<dyn BackupBackend>> {
    let secrets = crate::infra::secrets::global();
    let kind = secrets.get_or("CO_BACKUP_BACKEND", "local").to_lowercase();

    match kind.as_str() {
        "disabled" | "off" | "none" => None,
        "local" | "" => {
            let backup_dir = secrets.get_or("CO_BACKUP_DIR", "/data/backups/");
            let backup_dir = if backup_dir.starts_with('/') {
                PathBuf::from(backup_dir)
            } else {
                data_dir.join(backup_dir)
            };
            Some(Box::new(local_fs::LocalFsBackend::new(backup_dir)))
        }
        #[cfg(feature = "backup-s3")]
        "s3" => Some(Box::new(s3::S3Backend::new())),
        #[cfg(feature = "backup-r2")]
        "r2" => Some(Box::new(r2::R2Backend::new())),
        #[cfg(feature = "backup-fly")]
        "fly" => Some(Box::new(fly::FlyVolumeBackend::new())),
        #[cfg(feature = "backup-gcs")]
        "gcs" => Some(Box::new(gcs::GcsBackend::new())),
        other => {
            tracing::warn!(
                "CO_BACKUP_BACKEND={other} is unknown or feature not compiled; falling back to local"
            );
            let backup_dir = data_dir.join("backups");
            Some(Box::new(local_fs::LocalFsBackend::new(backup_dir)))
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot builder: creates the .tar.gz in a temp directory
// ---------------------------------------------------------------------------

/// Build a snapshot tarball from `data_dir` and return a `Snapshot` ready for
/// `backend.put()`.  The tarball is placed in `tmp_dir` and must be cleaned up
/// by the caller after `put()` completes.
pub fn build_snapshot(
    data_dir: &Path,
    tmp_dir: &Path,
    backend_name: &str,
) -> anyhow::Result<Snapshot> {
    let now = Utc::now();

    // Collect universe keys from data_dir/universes/
    let universes_path = data_dir.join("universes");
    let mut universe_keys: Vec<String> = Vec::new();
    if universes_path.exists() {
        for entry in std::fs::read_dir(&universes_path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                universe_keys.push(name.to_string());
            }
        }
    }
    universe_keys.sort();

    // Get DB schema version
    let schema_version = read_schema_version(data_dir).unwrap_or(0);

    // Build manifest
    let manifest = serde_json::json!({
        "created_at": now.to_rfc3339(),
        "co_version": env!("CARGO_PKG_VERSION"),
        "schema_version": schema_version,
        "universes": universe_keys,
        "backup_backend": backend_name,
    });

    // Write tarball
    let sha_placeholder = "tmp";
    let tmp_tarball = tmp_dir.join(format!("snapshot-tmp-{sha_placeholder}.tar.gz"));

    {
        let file = std::fs::File::create(&tmp_tarball)?;
        let gz_encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar_builder = tar::Builder::new(gz_encoder);

        // Add manifest.json
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest_bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar_builder.append_data(&mut header, "manifest.json", manifest_bytes.as_slice())?;

        // Add meta.db if it exists
        let meta_db = data_dir.join("meta.db");
        if meta_db.exists() {
            tar_builder.append_path_with_name(&meta_db, "meta.db")?;
        } else {
            // Try legacy co.db
            let co_db = data_dir.join("co.db");
            if co_db.exists() {
                tar_builder.append_path_with_name(&co_db, "meta.db")?;
            }
        }

        // Add universe files
        for key in &universe_keys {
            let universe_dir = universes_path.join(key);
            append_dir_recursive(&mut tar_builder, &universe_dir, &format!("universes/{key}"))?;
        }

        tar_builder.finish()?;
    }

    // Compute SHA-256 of the tarball
    let sha256 = compute_sha256(&tmp_tarball)?;
    let short_sha = &sha256[..16];

    // Final filename includes timestamp + sha
    let ts = now.format("%Y%m%dT%H%M%SZ");
    let final_name = format!("snapshot-{ts}-{short_sha}.tar.gz");
    let final_path = tmp_dir.join(&final_name);
    std::fs::rename(&tmp_tarball, &final_path)?;

    let bytes = std::fs::metadata(&final_path)?.len();

    Ok(Snapshot {
        created_at: now,
        manifest_json: manifest,
        tarball_path: final_path,
        bytes,
        sha256,
    })
}

fn read_schema_version(data_dir: &Path) -> anyhow::Result<i64> {
    let db_path = {
        let meta = data_dir.join("meta.db");
        if meta.exists() {
            meta
        } else {
            data_dir.join("co.db")
        }
    };
    if !db_path.exists() {
        return Ok(0);
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let v: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap_or(0);
    Ok(v)
}

fn append_dir_recursive<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    src: &Path,
    tar_prefix: &str,
) -> anyhow::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let tar_path = format!("{tar_prefix}/{}", entry.file_name().to_string_lossy());
        if ft.is_dir() {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_cksum();
            builder.append_data(&mut header, &tar_path, std::io::empty())?;
            append_dir_recursive(builder, &entry.path(), &tar_path)?;
        } else if ft.is_file() {
            builder.append_path_with_name(entry.path(), &tar_path)?;
        }
    }
    Ok(())
}

fn compute_sha256(path: &Path) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn snapshot_id_display() {
        let id = SnapshotId("snapshot-20260605T120000Z-abcd1234".to_string());
        assert_eq!(id.to_string(), "snapshot-20260605T120000Z-abcd1234");
    }

    #[test]
    fn build_snapshot_empty_data_dir() {
        let tmp = tempdir().unwrap();
        // data_dir with only meta.db missing — snapshot should still succeed
        let snapshot = build_snapshot(tmp.path(), tmp.path(), "local");
        assert!(snapshot.is_ok(), "empty data_dir should produce a snapshot");
        let s = snapshot.unwrap();
        assert!(!s.sha256.is_empty());
        assert!(s.bytes > 0);
        assert!(s.tarball_path.exists());
    }

    #[test]
    fn build_snapshot_manifest_includes_backend_name() {
        let tmp = tempdir().unwrap();
        let s = build_snapshot(tmp.path(), tmp.path(), "local").unwrap();
        assert_eq!(s.manifest_json["backup_backend"], "local");
        assert!(s.manifest_json["co_version"].is_string());
        assert!(s.manifest_json["created_at"].is_string());
    }

    #[test]
    fn backend_from_env_disabled() {
        // SAFETY: test-only, single-threaded context.
        unsafe { std::env::set_var("CO_BACKUP_BACKEND", "disabled") };
        let result = backend_from_env(Path::new("/data"));
        unsafe { std::env::remove_var("CO_BACKUP_BACKEND") };
        assert!(result.is_none());
    }

    #[test]
    fn local_fs_backend_name_is_local() {
        let tmp = tempdir().unwrap();
        let backend = local_fs::LocalFsBackend::new(tmp.path().join("backups"));
        assert_eq!(backend.name(), "local");
    }
}
