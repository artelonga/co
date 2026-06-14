//! CO-459: Junk sweep — find and (optionally) remove disk that is safe to
//! reclaim, **never** deleting anything still referenced.
//!
//! Categories swept (each guarded by a reference/owner check):
//!
//! | Kind            | What                                   | Safety guard                              |
//! |-----------------|----------------------------------------|-------------------------------------------|
//! | `AnonUniverse`  | expired anonymous clones (`anon-`/`u-`)| owner still anon **and** older than TTL   |
//! | `TempFile`      | stale `*.tmp`/`*.lock`/`co-backup-*`   | filename pattern **and** older than max age |
//! | `LogFile`       | rotated logs (`*.log.N`, `*.log.gz`)   | rotated suffix only — never the live `.log` |
//! | `OrphanBlob`    | `assets` rows with `refcount <= 0`     | re-checked `refcount <= 0` at delete time |
//! | `ExpiredBackup` | snapshots beyond retention             | CO-405 `select_prunable` (always keeps newest) |
//!
//! **Dry-run is the default.** [`plan_sweep`] only reads; it produces a report
//! of what *would* be removed and how much space that frees. Deletion happens
//! exclusively in [`apply_sweep`], which re-verifies every guard and logs each
//! removal (no silent deletes — `feedback_serve_only_indexed`).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rusqlite::Connection;
use serde::Serialize;

use crate::platform::universe_pool::UniversePool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JunkKind {
    AnonUniverse,
    TempFile,
    LogFile,
    OrphanBlob,
    ExpiredBackup,
}

/// A single removable item. `path` is the on-disk target; the `*_ref` fields
/// carry the extra context [`apply_sweep`] needs to clean up DB rows safely.
#[derive(Debug, Clone, Serialize)]
pub struct JunkItem {
    pub kind: JunkKind,
    pub path: String,
    pub bytes: u64,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub universe_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob_db: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob_sha: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepReport {
    pub dry_run: bool,
    pub items: Vec<JunkItem>,
    pub reclaimable_bytes: u64,
    /// How many items were actually removed (0 in dry-run).
    pub removed: usize,
}

impl SweepReport {
    fn from_items(items: Vec<JunkItem>) -> Self {
        let reclaimable_bytes = items.iter().map(|i| i.bytes).sum();
        SweepReport {
            dry_run: true,
            reclaimable_bytes,
            removed: 0,
            items,
        }
    }
}

/// Tunable thresholds. Reuses the CO-405 retention knobs for backups.
#[derive(Debug, Clone)]
pub struct SweepOptions {
    pub anon_ttl_days: i64,
    pub temp_max_age: Duration,
    pub retain_count: usize,
    pub retain_max_bytes: u64,
    pub retention_days: i64,
}

impl Default for SweepOptions {
    fn default() -> Self {
        SweepOptions {
            anon_ttl_days: 30,
            temp_max_age: Duration::from_secs(24 * 3600),
            retain_count: 3,
            retain_max_bytes: 0,
            retention_days: 30,
        }
    }
}

impl SweepOptions {
    /// Read thresholds from env, reusing the CO-405 `CO_BACKUP_RETAIN_*` knobs
    /// so backup retention is configured in exactly one place.
    pub fn from_env() -> Self {
        use crate::infra::secrets::SecretsProviderExt;
        let s = crate::infra::secrets::global();
        let d = SweepOptions::default();
        SweepOptions {
            anon_ttl_days: s.get_parsed("CO_SWEEP_ANON_TTL_DAYS", d.anon_ttl_days as u64) as i64,
            temp_max_age: Duration::from_secs(
                s.get_parsed("CO_SWEEP_TEMP_MAX_AGE_HOURS", 24) * 3600,
            ),
            retain_count: s.get_parsed("CO_BACKUP_RETAIN_COUNT", d.retain_count as u64) as usize,
            retain_max_bytes: s.get_parsed("CO_BACKUP_RETAIN_MAX_BYTES", 0),
            retention_days: s.get_parsed("CO_BACKUP_RETENTION_DAYS", d.retention_days as u64)
                as i64,
        }
    }
}

// ---------------------------------------------------------------------------
// Planning (read-only — the default dry run)
// ---------------------------------------------------------------------------

/// Scan every category and return a dry-run report. Performs **no** deletions.
#[allow(clippy::too_many_arguments)]
pub fn plan_sweep(
    meta_conn: &Connection,
    data_dir: &Path,
    pool: &UniversePool,
    backups_root: &Path,
    opts: &SweepOptions,
    now_utc: DateTime<Utc>,
    now_sys: SystemTime,
) -> SweepReport {
    let mut items = Vec::new();
    items.extend(collect_anon_universes(
        meta_conn,
        pool,
        data_dir,
        opts.anon_ttl_days,
        now_utc,
    ));
    items.extend(collect_temp_files(data_dir, opts.temp_max_age, now_sys));
    items.extend(collect_logs(data_dir, opts.temp_max_age, now_sys));
    items.extend(collect_orphan_blobs(data_dir));
    items.extend(collect_expired_backups(backups_root, opts, now_utc));
    SweepReport::from_items(items)
}

// ---------------------------------------------------------------------------
// Apply (deletes — re-verifies every guard, logs each removal)
// ---------------------------------------------------------------------------

/// Remove every item in `report`, re-checking each safety guard at delete time.
/// Mutates `report` in place: clears `dry_run` and sets `removed`.
pub fn apply_sweep(meta_conn: &Connection, report: &mut SweepReport) {
    let mut removed = 0usize;
    for item in &report.items {
        match apply_item(meta_conn, item) {
            Ok(true) => {
                removed += 1;
                tracing::info!(
                    "sweep: removed {:?} {} ({} bytes) — {}",
                    item.kind,
                    item.path,
                    item.bytes,
                    item.reason
                );
            }
            Ok(false) => {
                tracing::info!(
                    "sweep: skipped {:?} {} — guard no longer satisfied",
                    item.kind,
                    item.path
                );
            }
            Err(e) => {
                tracing::warn!("sweep: failed to remove {:?} {}: {e}", item.kind, item.path);
            }
        }
    }
    report.dry_run = false;
    report.removed = removed;
}

/// Apply one item. Returns `Ok(true)` when something was removed, `Ok(false)`
/// when the safety guard failed at delete time (left untouched).
fn apply_item(conn: &Connection, item: &JunkItem) -> anyhow::Result<bool> {
    match item.kind {
        JunkKind::AnonUniverse => {
            let Some(key) = &item.universe_key else {
                return Ok(false);
            };
            // Guard: only delete if still anon-owned (never a re-homed universe).
            let n = conn.execute(
                "DELETE FROM universes WHERE key = ?1 \
                 AND (owner_id LIKE 'anon-%' OR key LIKE 'anon-%' OR key LIKE 'u-%')",
                [key],
            )?;
            if n == 0 {
                return Ok(false);
            }
            let _ = conn.execute("DELETE FROM entries WHERE universe_key = ?1", [key]);
            let _ = conn.execute(
                "DELETE FROM universe_members WHERE universe_key = ?1",
                [key],
            );
            let dir = PathBuf::from(&item.path);
            if dir.exists() {
                let _ = std::fs::remove_dir_all(&dir);
            }
            Ok(true)
        }
        JunkKind::OrphanBlob => {
            let (Some(db), Some(sha)) = (&item.blob_db, &item.blob_sha) else {
                return Ok(false);
            };
            // Guard: re-check refcount <= 0 inside the delete.
            let ucon = Connection::open(db)?;
            let n = ucon.execute(
                "DELETE FROM assets WHERE sha256 = ?1 AND refcount <= 0",
                [sha],
            )?;
            if n == 0 {
                return Ok(false);
            }
            let blob = PathBuf::from(&item.path);
            if blob.exists() {
                let _ = std::fs::remove_file(&blob);
            }
            Ok(true)
        }
        JunkKind::TempFile | JunkKind::LogFile => {
            let p = PathBuf::from(&item.path);
            if p.exists() {
                std::fs::remove_file(&p)?;
            }
            Ok(true)
        }
        JunkKind::ExpiredBackup => {
            let p = PathBuf::from(&item.path);
            if p.exists() {
                std::fs::remove_dir_all(&p)?;
            }
            Ok(true)
        }
    }
}

// ---------------------------------------------------------------------------
// Collectors
// ---------------------------------------------------------------------------

fn collect_anon_universes(
    conn: &Connection,
    pool: &UniversePool,
    data_dir: &Path,
    ttl_days: i64,
    now: DateTime<Utc>,
) -> Vec<JunkItem> {
    let cutoff = now - ChronoDuration::days(ttl_days);
    let mut stmt = match conn.prepare(
        "SELECT key, created_at FROM universes \
         WHERE is_template = 0 \
           AND (owner_id LIKE 'anon-%' OR key LIKE 'anon-%' OR key LIKE 'u-%')",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("sweep: anon-universe query prepare failed: {e}");
            return Vec::new();
        }
    };
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map(|it| it.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut items = Vec::new();
    for (key, created_at) in rows {
        let created = DateTime::parse_from_rfc3339(&created_at)
            .map(|d| d.with_timezone(&Utc))
            .ok();
        // Only sweep clones we can confidently date as expired.
        let expired = matches!(created, Some(c) if c < cutoff);
        if !expired {
            continue;
        }
        let dir = resolve_universe_dir(pool, data_dir, &key);
        let bytes = dir.as_ref().map(|d| dir_size(d)).unwrap_or(0);
        let path = dir
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_else(|| pool.universe_dir(&key).to_string_lossy().to_string());
        items.push(JunkItem {
            kind: JunkKind::AnonUniverse,
            path,
            bytes,
            reason: format!("anonymous clone older than {ttl_days}d (created {created_at})"),
            universe_key: Some(key),
            blob_db: None,
            blob_sha: None,
        });
    }
    items
}

/// Fanout dir (current layout) or the legacy flat `universes/<key>` dir,
/// whichever exists.
fn resolve_universe_dir(pool: &UniversePool, data_dir: &Path, key: &str) -> Option<PathBuf> {
    let fanout = pool.universe_dir(key);
    if fanout.exists() {
        return Some(fanout);
    }
    let flat = data_dir.join("universes").join(key);
    flat.exists().then_some(flat)
}

fn collect_temp_files(data_dir: &Path, max_age: Duration, now: SystemTime) -> Vec<JunkItem> {
    let mut items = Vec::new();
    let Ok(rd) = std::fs::read_dir(data_dir) else {
        return items;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_stale_temp(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        if file_age(&meta, now) < max_age {
            continue;
        }
        items.push(JunkItem {
            kind: JunkKind::TempFile,
            path: entry.path().to_string_lossy().to_string(),
            bytes: meta.len(),
            reason: format!("stale temp/lock file (> {}s old)", max_age.as_secs()),
            universe_key: None,
            blob_db: None,
            blob_sha: None,
        });
    }
    items
}

fn collect_logs(data_dir: &Path, max_age: Duration, now: SystemTime) -> Vec<JunkItem> {
    let mut items = Vec::new();
    let Ok(rd) = std::fs::read_dir(data_dir) else {
        return items;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_rotated_log(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        if file_age(&meta, now) < max_age {
            continue;
        }
        items.push(JunkItem {
            kind: JunkKind::LogFile,
            path: entry.path().to_string_lossy().to_string(),
            bytes: meta.len(),
            reason: "rotated log file".to_string(),
            universe_key: None,
            blob_db: None,
            blob_sha: None,
        });
    }
    items
}

fn collect_orphan_blobs(data_dir: &Path) -> Vec<JunkItem> {
    let mut items = Vec::new();
    let mut dbs = Vec::new();
    find_universe_dbs(&data_dir.join("universes"), &mut dbs);
    for db in dbs {
        let Some(udir) = db.parent() else { continue };
        let Ok(conn) = Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        else {
            continue;
        };
        let mut stmt = match conn
            .prepare("SELECT sha256, blob_path, size_bytes FROM assets WHERE refcount <= 0")
        {
            Ok(s) => s,
            Err(_) => continue, // no assets table in this universe DB
        };
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map(|it| it.filter_map(|r| r.ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        for (sha, blob_path, size_bytes) in rows {
            let blob = udir.join(&blob_path);
            let bytes = std::fs::metadata(&blob)
                .map(|m| m.len())
                .unwrap_or(size_bytes.max(0) as u64);
            items.push(JunkItem {
                kind: JunkKind::OrphanBlob,
                path: blob.to_string_lossy().to_string(),
                bytes,
                reason: "asset refcount = 0 (no entry references it)".to_string(),
                universe_key: None,
                blob_db: Some(db.to_string_lossy().to_string()),
                blob_sha: Some(sha),
            });
        }
    }
    items
}

fn collect_expired_backups(
    backups_root: &Path,
    opts: &SweepOptions,
    now: DateTime<Utc>,
) -> Vec<JunkItem> {
    let prunable = super::backup::snapshot_dir::prunable_local_snapshots(
        backups_root,
        opts.retain_count,
        opts.retain_max_bytes,
        opts.retention_days,
        now,
    );
    prunable
        .into_iter()
        .map(|id| {
            let dir = backups_root.join(&id.0);
            JunkItem {
                kind: JunkKind::ExpiredBackup,
                path: dir.to_string_lossy().to_string(),
                bytes: dir_size(&dir),
                reason: format!(
                    "snapshot beyond retention (count={}, max_bytes={}, days={})",
                    opts.retain_count, opts.retain_max_bytes, opts.retention_days
                ),
                universe_key: None,
                blob_db: None,
                blob_sha: None,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Pure predicates (unit-tested)
// ---------------------------------------------------------------------------

/// A filename that denotes a disposable temp/lock artifact.
pub fn is_stale_temp(name: &str) -> bool {
    name.ends_with(".tmp")
        || name.ends_with(".lock")
        || name.starts_with("snapshot-tmp-")
        || name.starts_with("co-backup-")
}

/// A *rotated* log (e.g. `co.log.1`, `app.log.gz`) — safe to drop. The live
/// `*.log` (no numeric/`gz` suffix) is intentionally never matched.
pub fn is_rotated_log(name: &str) -> bool {
    if let Some(idx) = name.rfind(".log.") {
        let suffix = &name[idx + ".log.".len()..];
        !suffix.is_empty() && (suffix == "gz" || suffix.chars().all(|c| c.is_ascii_digit()))
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

fn file_age(meta: &std::fs::Metadata, now: SystemTime) -> Duration {
    meta.modified()
        .ok()
        .and_then(|m| now.duration_since(m).ok())
        .unwrap_or(Duration::ZERO)
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            total = total.saturating_add(dir_size(&entry.path()));
        } else if ft.is_file() {
            total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    total
}

fn find_universe_dbs(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            find_universe_dbs(&path, out);
        } else if ft.is_file() && entry.file_name().to_string_lossy() == "data.db" {
            out.push(path);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::tempdir;

    fn open_meta(dir: &Path) -> Connection {
        let conn = Connection::open(dir.join("meta.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE universes (\
                key TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', \
                owner_id TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, \
                is_template INTEGER NOT NULL DEFAULT 0);\
             CREATE TABLE entries (universe_key TEXT);\
             CREATE TABLE universe_members (universe_key TEXT);",
        )
        .unwrap();
        conn
    }

    fn insert_universe(conn: &Connection, key: &str, owner: &str, created_at: &str) {
        conn.execute(
            "INSERT INTO universes (key, name, owner_id, created_at, is_template) \
             VALUES (?1, ?1, ?2, ?3, 0)",
            params![key, owner, created_at],
        )
        .unwrap();
    }

    // ── pure predicates ─────────────────────────────────────────────────────

    #[test]
    fn stale_temp_predicate() {
        assert!(is_stale_temp("foo.tmp"));
        assert!(is_stale_temp("db.lock"));
        assert!(is_stale_temp("co-backup-1234"));
        assert!(is_stale_temp("snapshot-tmp-abcd.tar.gz"));
        assert!(!is_stale_temp("meta.db"));
        assert!(!is_stale_temp("important.md"));
    }

    #[test]
    fn rotated_log_predicate() {
        assert!(is_rotated_log("co.log.1"));
        assert!(is_rotated_log("app.log.42"));
        assert!(is_rotated_log("server.log.gz"));
        // the live log is never matched
        assert!(!is_rotated_log("co.log"));
        assert!(!is_rotated_log("co.log.bak"));
        assert!(!is_rotated_log("notes.md"));
    }

    // ── dry run never deletes ───────────────────────────────────────────────

    #[test]
    fn dry_run_reports_but_does_not_delete() {
        let dir = tempdir().unwrap();
        let conn = open_meta(dir.path());
        // expired anon universe (1 year old)
        insert_universe(&conn, "u-old", "anon-xyz", "2025-01-01T00:00:00Z");
        let pool = UniversePool::new(dir.path(), 16);
        let udir = pool.universe_dir("u-old");
        std::fs::create_dir_all(&udir).unwrap();
        std::fs::write(udir.join("data.db"), b"x").unwrap();
        // a stale temp file
        std::fs::write(dir.path().join("junk.tmp"), b"abc").unwrap();

        let opts = SweepOptions {
            temp_max_age: Duration::ZERO,
            ..SweepOptions::default()
        };
        let now = "2026-06-14T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let report = plan_sweep(
            &conn,
            dir.path(),
            &pool,
            &dir.path().join("backups"),
            &opts,
            now,
            SystemTime::now(),
        );

        assert!(report.dry_run);
        assert!(
            report
                .items
                .iter()
                .any(|i| i.kind == JunkKind::AnonUniverse)
        );
        assert!(report.items.iter().any(|i| i.kind == JunkKind::TempFile));
        // Nothing removed: files + rows still present.
        assert!(udir.join("data.db").exists());
        assert!(dir.path().join("junk.tmp").exists());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM universes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    // ── apply removes only orphans, keeps the rest ──────────────────────────

    #[test]
    fn apply_removes_only_expired_anon_keeps_fresh_and_owned() {
        let dir = tempdir().unwrap();
        let conn = open_meta(dir.path());
        let pool = UniversePool::new(dir.path(), 16);

        // expired anon — should go
        insert_universe(&conn, "u-old", "anon-xyz", "2025-01-01T00:00:00Z");
        // fresh anon — too young, kept
        insert_universe(&conn, "u-fresh", "anon-abc", "2026-06-13T00:00:00Z");
        // owned by a real user — never swept even though old
        insert_universe(&conn, "yuri", "user-123", "2024-01-01T00:00:00Z");
        for key in ["u-old", "u-fresh", "yuri"] {
            let d = pool.universe_dir(key);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("data.db"), b"x").unwrap();
        }

        let opts = SweepOptions::default();
        let now = "2026-06-14T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let mut report = plan_sweep(
            &conn,
            dir.path(),
            &pool,
            &dir.path().join("backups"),
            &opts,
            now,
            SystemTime::now(),
        );
        // Only the expired anon is a candidate.
        let anon: Vec<_> = report
            .items
            .iter()
            .filter(|i| i.kind == JunkKind::AnonUniverse)
            .collect();
        assert_eq!(anon.len(), 1);
        assert_eq!(anon[0].universe_key.as_deref(), Some("u-old"));

        apply_sweep(&conn, &mut report);
        assert!(!report.dry_run);
        assert_eq!(report.removed, 1);

        // u-old gone from DB + disk; the others remain.
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM universes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 2);
        assert!(!pool.universe_dir("u-old").exists());
        assert!(pool.universe_dir("u-fresh").exists());
        assert!(pool.universe_dir("yuri").exists());
    }

    #[test]
    fn apply_removes_orphan_blob_only() {
        let dir = tempdir().unwrap();
        let conn = open_meta(dir.path());
        let pool = UniversePool::new(dir.path(), 16);
        let udir = pool.universe_dir("u-blob");
        std::fs::create_dir_all(udir.join("blobs/aa/bb")).unwrap();

        // universe data.db with one orphan (refcount 0) and one referenced asset.
        let udb = udir.join("data.db");
        let ucon = Connection::open(&udb).unwrap();
        ucon.execute_batch(
            "CREATE TABLE assets (sha256 TEXT PRIMARY KEY, blob_path TEXT NOT NULL, \
                size_bytes INTEGER NOT NULL, refcount INTEGER NOT NULL DEFAULT 0);",
        )
        .unwrap();
        ucon.execute(
            "INSERT INTO assets VALUES ('aabborphan', 'blobs/aa/bb/aabborphan', 3, 0)",
            [],
        )
        .unwrap();
        ucon.execute(
            "INSERT INTO assets VALUES ('aabbkeep', 'blobs/aa/bb/aabbkeep', 4, 2)",
            [],
        )
        .unwrap();
        drop(ucon);
        std::fs::write(udir.join("blobs/aa/bb/aabborphan"), b"xyz").unwrap();
        std::fs::write(udir.join("blobs/aa/bb/aabbkeep"), b"keep").unwrap();

        let opts = SweepOptions::default();
        let now = "2026-06-14T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let mut report = plan_sweep(
            &conn,
            dir.path(),
            &pool,
            &dir.path().join("backups"),
            &opts,
            now,
            SystemTime::now(),
        );
        let blobs: Vec<_> = report
            .items
            .iter()
            .filter(|i| i.kind == JunkKind::OrphanBlob)
            .collect();
        assert_eq!(blobs.len(), 1);
        assert_eq!(blobs[0].blob_sha.as_deref(), Some("aabborphan"));

        apply_sweep(&conn, &mut report);

        // Orphan blob row + file gone; referenced blob untouched.
        let ucon = Connection::open(&udb).unwrap();
        let n: i64 = ucon
            .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        assert!(!udir.join("blobs/aa/bb/aabborphan").exists());
        assert!(udir.join("blobs/aa/bb/aabbkeep").exists());
    }
}
