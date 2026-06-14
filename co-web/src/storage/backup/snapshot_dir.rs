//! CO-459: Reconstructable local snapshot — `/data/backups/<ts>/` directory with
//! a **per-file sha256 manifest**.
//!
//! Unlike the CO-365 tarball backup (a single `.tar.gz` with one whole-archive
//! hash, built from a *hot copy* of the live DBs), this snapshot is:
//!
//! 1. **Consistent**: `meta.db` and every per-universe `data.db` are copied via
//!    SQLite `VACUUM INTO` (a committed-state snapshot), never a hot byte copy
//!    of a WAL-mode file.
//! 2. **Verifiable**: `manifest.json` records a sha256 for every file, so a
//!    re-hash of the directory proves the snapshot is reconstructable
//!    (see [`verify_snapshot`]).
//! 3. **Retention-managed**: directory snapshots are pruned by the same
//!    count/size/age policy as CO-405 (reuses [`super::worker::select_prunable`]).
//!
//! The directory snapshots coexist with the CO-365 tarballs under the same
//! `CO_BACKUP_DIR`: tarballs are `*.tar.gz` files, these are `<ts>/` directories
//! carrying a `manifest.json`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{SnapshotId, SnapshotMeta};

/// One file inside a snapshot directory, addressed by its path relative to the
/// snapshot root.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    /// Path relative to the snapshot directory (e.g. `meta.db`,
    /// `universes/ab/cd/key/data.db`).
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// The `manifest.json` written at the root of every snapshot directory. The
/// manifest itself is **not** listed in `files` (it is the index, not content).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalManifest {
    pub created_at: String,
    pub co_version: String,
    pub schema_version: i64,
    pub total_bytes: u64,
    pub files: Vec<FileEntry>,
}

/// Result of re-hashing a snapshot against its manifest.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    /// True when every manifest file is present and its hash matches.
    pub verified: bool,
    pub checked: usize,
    /// Files whose on-disk hash differs from the manifest.
    pub mismatches: Vec<String>,
    /// Files listed in the manifest but absent from the directory.
    pub missing: Vec<String>,
}

// ---------------------------------------------------------------------------
// Backups root resolution (shares CO_BACKUP_DIR with the CO-365 tarballs)
// ---------------------------------------------------------------------------

/// Resolve the backups root directory the same way the local tarball backend
/// does: `CO_BACKUP_DIR` (default `/data/backups/`), relative paths joined onto
/// `data_dir`.
pub fn backups_root(data_dir: &Path) -> PathBuf {
    let dir = crate::infra::secrets::global().get_or("CO_BACKUP_DIR", "/data/backups/");
    if dir.starts_with('/') {
        PathBuf::from(dir)
    } else {
        data_dir.join(dir)
    }
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// Build a reconstructable snapshot of `data_dir` under
/// `backups_root/<ts>/` and return `(snapshot_dir, manifest)`.
///
/// `now` is injected so callers (and tests) control the timestamp. `meta.db`
/// and every `*.db` under `universes/` are copied with `VACUUM INTO`; all other
/// files are byte-copied. A sha256 is recorded for each.
pub fn build_local_snapshot(
    data_dir: &Path,
    backups_root: &Path,
    now: DateTime<Utc>,
) -> anyhow::Result<(PathBuf, LocalManifest)> {
    let ts = now.format("%Y%m%dT%H%M%SZ").to_string();
    let snapshot_dir = backups_root.join(&ts);
    if snapshot_dir.exists() {
        anyhow::bail!(
            "snapshot directory already exists: {}",
            snapshot_dir.display()
        );
    }
    std::fs::create_dir_all(&snapshot_dir)?;

    // 1. meta.db (consistent copy via VACUUM INTO).
    let meta_src = {
        let meta = data_dir.join("meta.db");
        if meta.exists() {
            Some(meta)
        } else {
            let legacy = data_dir.join("co.db");
            legacy.exists().then_some(legacy)
        }
    };
    if let Some(src) = &meta_src {
        copy_db_consistent(src, &snapshot_dir.join("meta.db"))?;
    }

    // 2. universes/ tree — VACUUM INTO for *.db, byte-copy for everything else.
    let universes_src = data_dir.join("universes");
    if universes_src.exists() {
        copy_tree(&universes_src, &snapshot_dir.join("universes"))?;
    }

    // 3. Hash every file now present in the snapshot (excluding the manifest).
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    collect_files(&snapshot_dir, &snapshot_dir, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for f in &files {
        total_bytes = total_bytes.saturating_add(f.bytes);
    }

    let schema_version = read_schema_version(&snapshot_dir.join("meta.db")).unwrap_or(0);
    let manifest = LocalManifest {
        created_at: now.to_rfc3339(),
        co_version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version,
        total_bytes,
        files,
    };

    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    std::fs::write(snapshot_dir.join("manifest.json"), manifest_bytes)?;

    Ok((snapshot_dir, manifest))
}

/// Copy a SQLite database to `dest` consistently via `VACUUM INTO`. Falls back
/// to a byte copy (with a warning) if the source is not a usable SQLite DB.
fn copy_db_consistent(src: &Path, dest: &Path) -> anyhow::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match vacuum_into(src, dest) {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!(
                "snapshot: VACUUM INTO failed for {} ({e}); falling back to byte copy",
                src.display()
            );
            // VACUUM INTO may have left a partial file — remove before copying.
            let _ = std::fs::remove_file(dest);
            std::fs::copy(src, dest)?;
            Ok(())
        }
    }
}

/// `VACUUM INTO 'dest'` from a read-only connection on `src`. `dest` must not
/// already exist (SQLite requirement).
fn vacuum_into(src: &Path, dest: &Path) -> anyhow::Result<()> {
    let conn =
        rusqlite::Connection::open_with_flags(src, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let dest_str = dest
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("non-UTF-8 snapshot path"))?;
    // VACUUM cannot run with a bound parameter; escape single quotes instead.
    let escaped = dest_str.replace('\'', "''");
    conn.execute_batch(&format!("VACUUM INTO '{escaped}';"))?;
    Ok(())
}

/// Recursively copy `src` into `dest`. `*.db` files go through `VACUUM INTO`;
/// WAL/SHM sidecars are skipped (their committed state is already captured by
/// the vacuum); everything else is byte-copied.
fn copy_tree(src: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let child_src = entry.path();
        let child_dest = dest.join(&name);
        if ft.is_dir() {
            copy_tree(&child_src, &child_dest)?;
        } else if ft.is_file() {
            if name_str.ends_with("-wal") || name_str.ends_with("-shm") {
                continue; // committed state captured by VACUUM INTO of the .db
            }
            if name_str.ends_with(".db") {
                copy_db_consistent(&child_src, &child_dest)?;
            } else {
                std::fs::copy(&child_src, &child_dest)?;
            }
        }
    }
    Ok(())
}

/// Walk `dir`, recording a [`FileEntry`] (relative to `root`) for every file
/// except the top-level `manifest.json`.
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<FileEntry>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let path = entry.path();
        if ft.is_dir() {
            collect_files(root, &path, out)?;
        } else if ft.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "manifest.json" {
                continue;
            }
            let bytes = entry.metadata()?.len();
            let sha256 = sha256_file(&path)?;
            out.push(FileEntry {
                path: rel,
                sha256,
                bytes,
            });
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn read_schema_version(db_path: &Path) -> anyhow::Result<i64> {
    if !db_path.exists() {
        return Ok(0);
    }
    let conn =
        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let v: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap_or(0);
    Ok(v)
}

// ---------------------------------------------------------------------------
// Verify
// ---------------------------------------------------------------------------

/// Re-hash every file listed in `<snapshot_dir>/manifest.json` and compare to
/// the recorded sha256. This is the proof-of-reconstruction check: it passes
/// only when nothing is missing and nothing has drifted.
pub fn verify_snapshot(snapshot_dir: &Path) -> anyhow::Result<VerifyReport> {
    let manifest_bytes = std::fs::read(snapshot_dir.join("manifest.json"))?;
    let manifest: LocalManifest = serde_json::from_slice(&manifest_bytes)?;

    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    for f in &manifest.files {
        let p = snapshot_dir.join(&f.path);
        if !p.exists() {
            missing.push(f.path.clone());
            continue;
        }
        match sha256_file(&p) {
            Ok(h) if h == f.sha256 => {}
            Ok(_) => mismatches.push(f.path.clone()),
            Err(_) => missing.push(f.path.clone()),
        }
    }

    Ok(VerifyReport {
        verified: mismatches.is_empty() && missing.is_empty(),
        checked: manifest.files.len(),
        mismatches,
        missing,
    })
}

// ---------------------------------------------------------------------------
// Listing + retention (reuses CO-405 select_prunable)
// ---------------------------------------------------------------------------

/// List directory snapshots under `backups_root`, newest-first. Only
/// directories carrying a readable `manifest.json` are returned; tarballs and
/// stray files are ignored.
pub fn list_local_snapshots(backups_root: &Path) -> Vec<SnapshotMeta> {
    let mut metas = Vec::new();
    let Ok(rd) = std::fs::read_dir(backups_root) else {
        return metas;
    };
    for entry in rd.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let dir = entry.path();
        let manifest_path = dir.join("manifest.json");
        let Ok(bytes) = std::fs::read(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<LocalManifest>(&bytes) else {
            continue;
        };
        let created_at = DateTime::parse_from_rfc3339(&manifest.created_at)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        metas.push(SnapshotMeta {
            id: SnapshotId(entry.file_name().to_string_lossy().to_string()),
            created_at,
            bytes: manifest.total_bytes,
            sha256: String::new(),
        });
    }
    metas.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    metas
}

/// IDs (directory names) of snapshots that exceed the count/size/age retention
/// policy. Reuses the CO-405 [`super::worker::select_prunable`] decision logic.
pub fn prunable_local_snapshots(
    backups_root: &Path,
    retain_count: usize,
    retain_max_bytes: u64,
    retention_days: i64,
    now: DateTime<Utc>,
) -> Vec<SnapshotId> {
    let metas = list_local_snapshots(backups_root);
    super::worker::select_prunable(&metas, retain_count, retain_max_bytes, retention_days, now)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Create a minimal SQLite DB with a `schema_version` table so the snapshot
    /// path exercises VACUUM INTO + version read.
    fn make_db(path: &Path, version: i64) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER);\
             CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [version],
        )
        .unwrap();
        conn.execute("INSERT INTO t (v) VALUES ('hello')", [])
            .unwrap();
    }

    #[test]
    fn build_then_verify_roundtrips() {
        let data = tempdir().unwrap();
        let backups = tempdir().unwrap();
        make_db(&data.path().join("meta.db"), 13);
        // a per-universe data.db
        let udir = data.path().join("universes/ab/cd/u-key");
        std::fs::create_dir_all(&udir).unwrap();
        make_db(&udir.join("data.db"), 7);
        // a non-db blob file
        std::fs::create_dir_all(udir.join("blobs/aa/bb")).unwrap();
        std::fs::write(udir.join("blobs/aa/bb/deadbeef"), b"binary-bytes").unwrap();

        let now = "2026-06-14T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let (snap_dir, manifest) = build_local_snapshot(data.path(), backups.path(), now).unwrap();

        assert!(snap_dir.join("meta.db").exists());
        assert!(snap_dir.join("universes/ab/cd/u-key/data.db").exists());
        assert!(
            snap_dir
                .join("universes/ab/cd/u-key/blobs/aa/bb/deadbeef")
                .exists()
        );
        assert_eq!(manifest.schema_version, 13);
        // meta.db + data.db + blob = 3 files
        assert_eq!(manifest.files.len(), 3);
        assert!(manifest.files.iter().all(|f| !f.sha256.is_empty()));

        let report = verify_snapshot(&snap_dir).unwrap();
        assert!(report.verified, "fresh snapshot must verify: {report:?}");
        assert_eq!(report.checked, 3);
    }

    #[test]
    fn verify_detects_tamper() {
        let data = tempdir().unwrap();
        let backups = tempdir().unwrap();
        make_db(&data.path().join("meta.db"), 1);
        let now = "2026-06-14T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let (snap_dir, _) = build_local_snapshot(data.path(), backups.path(), now).unwrap();

        // Tamper with a snapshot file after the manifest was written.
        std::fs::write(snap_dir.join("meta.db"), b"corrupted").unwrap();
        let report = verify_snapshot(&snap_dir).unwrap();
        assert!(!report.verified);
        assert_eq!(report.mismatches, vec!["meta.db".to_string()]);
    }

    #[test]
    fn verify_detects_missing_file() {
        let data = tempdir().unwrap();
        let backups = tempdir().unwrap();
        make_db(&data.path().join("meta.db"), 1);
        let now = "2026-06-14T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let (snap_dir, _) = build_local_snapshot(data.path(), backups.path(), now).unwrap();

        std::fs::remove_file(snap_dir.join("meta.db")).unwrap();
        let report = verify_snapshot(&snap_dir).unwrap();
        assert!(!report.verified);
        assert_eq!(report.missing, vec!["meta.db".to_string()]);
    }

    #[test]
    fn list_and_prune_respects_retention() {
        let backups = tempdir().unwrap();
        let data = tempdir().unwrap();
        make_db(&data.path().join("meta.db"), 1);

        // Three snapshots at distinct timestamps.
        let times = [
            "2026-06-10T00:00:00Z",
            "2026-06-12T00:00:00Z",
            "2026-06-14T00:00:00Z",
        ];
        for t in times {
            let now = t.parse::<DateTime<Utc>>().unwrap();
            build_local_snapshot(data.path(), backups.path(), now).unwrap();
        }

        let metas = list_local_snapshots(backups.path());
        assert_eq!(metas.len(), 3);
        // newest-first ordering
        assert!(metas[0].created_at > metas[1].created_at);

        // retain_count=1 → prune the two oldest (newest always kept).
        let now = "2026-06-14T01:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let prunable = prunable_local_snapshots(backups.path(), 1, 0, 30, now);
        assert_eq!(prunable.len(), 2);
        // The newest (20260614T000000Z) must not be in the prune list.
        assert!(!prunable.iter().any(|id| id.0 == "20260614T000000Z"));
    }
}
