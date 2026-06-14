//! CO-449: telemetry cold-tier archival to Parquet — keep all data, shrink `meta.db`.
//!
//! `telemetry_events` is append-only and high-volume; in prod (2026-06-14) it
//! dominates an 11 GB `meta.db` on a 20 GB volume at 71% full. The owner decision
//! is **not** to prune — every event is relevant — but to move the *cold* window
//! out of the OLTP database into a columnar, compressed, still-queryable format:
//! **Parquet (zstd)**, ~10–20× smaller than SQLite rows and readable by DuckDB
//! (`SELECT … FROM read_parquet('…/telemetry/**/*.parquet')`).
//!
//! ## Hot vs cold
//! - **Hot** (default 90d, `CO_TELEMETRY_HOT_DAYS`): stays in `telemetry_events`;
//!   the `/gestao/resumo` dashboards (CO-360) read it for low latency.
//! - **Cold** (any month strictly older than the hot window): exported to Parquet
//!   and deleted from `meta.db`. A month is only ever archived **whole**, so the
//!   hot window is never split.
//!
//! ## Safety: verify-before-delete, no 2× peak
//! Per month, oldest first: export → verify (Parquet row count == SQLite count,
//! plus sha256) → move into place → **only then** delete → reclaim freed pages
//! with `PRAGMA incremental_vacuum` (no full-VACUUM rebuild, which prod can't fit).
//! Every archived month is recorded in `telemetry_archives` for traceability and
//! dedupe, so the job is idempotent and resumable: re-running skips months already
//! listed and a crash mid-month leaves SQLite intact (delete is last).
//!
//! ## Local-first (reframe 2026-06-14)
//! Destination is the local filesystem (`<data_dir>/telemetry-archive/`) today;
//! uploading the same files to S3 (CO-81) under `telemetry/year=/month=/` is a
//! follow-up. The verify + delete + reclaim run against the local file, which is
//! what delivers the `meta.db` shrink now.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{ArrayRef, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use chrono::{DateTime, Datelike, Duration, Utc};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

/// Default hot-window size: events newer than this many days stay in `meta.db`.
pub const DEFAULT_HOT_DAYS: i64 = 90;

/// Inputs for one archival run. `now` is injected so the cold cutoff is
/// deterministic in tests (no `Utc::now()` dependence).
pub struct ArchiveConfig {
    /// Root directory for the Parquet partitions (`<root>/year=YYYY/month=MM/…`).
    pub archive_dir: PathBuf,
    /// Events in months strictly older than `now - hot_days` are archived.
    pub hot_days: i64,
    /// "Current time" for computing the cold cutoff and `archived_at`.
    pub now: DateTime<Utc>,
}

/// One month successfully exported and removed from `meta.db`.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchivedMonth {
    pub year: i32,
    pub month: u32,
    /// Path relative to `archive_dir` (also the `s3_key` once CO-81 lands).
    pub relative_path: String,
    pub rows: i64,
    pub sha256: String,
    pub bytes: u64,
}

/// Outcome of an archival run.
#[derive(Debug, Default)]
pub struct ArchiveReport {
    /// Months archived this run, oldest first.
    pub archived: Vec<ArchivedMonth>,
    /// Cold months skipped because they were already in `telemetry_archives`.
    pub skipped_existing: Vec<(i32, u32)>,
    /// Total rows moved out of `meta.db` this run.
    pub total_rows_archived: i64,
}

/// Parquet column layout — mirrors `telemetry_events` (CO-46 + CO-178 geo cols).
fn parquet_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("timestamp", DataType::Utf8, false),
        Field::new("visitor_token", DataType::Utf8, true),
        Field::new("user_id", DataType::Utf8, true),
        Field::new("session_id", DataType::Utf8, true),
        Field::new("event_type", DataType::Utf8, false),
        Field::new("event_name", DataType::Utf8, false),
        Field::new("universe_key", DataType::Utf8, true),
        Field::new("path", DataType::Utf8, true),
        Field::new("properties", DataType::Utf8, true),
        Field::new("duration_ms", DataType::Int64, true),
        Field::new("ip_hash", DataType::Utf8, true),
        Field::new("ua_device", DataType::Utf8, true),
        Field::new("ua_browser", DataType::Utf8, true),
        Field::new("ua_os", DataType::Utf8, true),
        Field::new("country", DataType::Utf8, true),
        Field::new("city", DataType::Utf8, true),
    ]))
}

/// All distinct `(year, month)` present in `telemetry_events`, oldest first.
/// `strftime` returns NULL for unparseable timestamps; those are excluded so a
/// malformed row never derails the run.
fn distinct_months(conn: &Connection) -> rusqlite::Result<Vec<(i32, u32)>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT CAST(strftime('%Y', timestamp) AS INTEGER) AS y, \
                         CAST(strftime('%m', timestamp) AS INTEGER) AS m \
         FROM telemetry_events \
         WHERE timestamp IS NOT NULL AND strftime('%Y', timestamp) IS NOT NULL \
         ORDER BY y, m",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i32>(0)?, r.get::<_, i32>(1)? as u32))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Months already archived (so a re-run skips them — idempotency + dedupe).
fn archived_months(conn: &Connection) -> rusqlite::Result<std::collections::HashSet<(i32, u32)>> {
    let mut stmt = conn.prepare("SELECT year, month FROM telemetry_archives")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i32>(0)?, r.get::<_, i32>(1)? as u32))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

/// Encode bytes as lowercase hex (avoids pulling in the `hex` crate).
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Count of rows in `telemetry_events` for one month.
fn month_row_count(conn: &Connection, year: i32, month: u32) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM telemetry_events \
         WHERE CAST(strftime('%Y', timestamp) AS INTEGER) = ?1 \
           AND CAST(strftime('%m', timestamp) AS INTEGER) = ?2",
        rusqlite::params![year, month],
        |r| r.get(0),
    )
}

/// Read one month's rows into an Arrow [`RecordBatch`].
fn month_record_batch(conn: &Connection, year: i32, month: u32) -> anyhow::Result<RecordBatch> {
    let mut stmt = conn.prepare(
        "SELECT id, timestamp, visitor_token, user_id, session_id, event_type, event_name, \
                universe_key, path, properties, duration_ms, ip_hash, ua_device, ua_browser, \
                ua_os, country, city \
         FROM telemetry_events \
         WHERE CAST(strftime('%Y', timestamp) AS INTEGER) = ?1 \
           AND CAST(strftime('%m', timestamp) AS INTEGER) = ?2 \
         ORDER BY id",
    )?;

    let mut ids: Vec<i64> = Vec::new();
    let mut timestamp: Vec<Option<String>> = Vec::new();
    let mut visitor_token: Vec<Option<String>> = Vec::new();
    let mut user_id: Vec<Option<String>> = Vec::new();
    let mut session_id: Vec<Option<String>> = Vec::new();
    let mut event_type: Vec<Option<String>> = Vec::new();
    let mut event_name: Vec<Option<String>> = Vec::new();
    let mut universe_key: Vec<Option<String>> = Vec::new();
    let mut path: Vec<Option<String>> = Vec::new();
    let mut properties: Vec<Option<String>> = Vec::new();
    let mut duration_ms: Vec<Option<i64>> = Vec::new();
    let mut ip_hash: Vec<Option<String>> = Vec::new();
    let mut ua_device: Vec<Option<String>> = Vec::new();
    let mut ua_browser: Vec<Option<String>> = Vec::new();
    let mut ua_os: Vec<Option<String>> = Vec::new();
    let mut country: Vec<Option<String>> = Vec::new();
    let mut city: Vec<Option<String>> = Vec::new();

    let mut rows = stmt.query(rusqlite::params![year, month])?;
    while let Some(r) = rows.next()? {
        ids.push(r.get(0)?);
        timestamp.push(r.get(1)?);
        visitor_token.push(r.get(2)?);
        user_id.push(r.get(3)?);
        session_id.push(r.get(4)?);
        event_type.push(r.get(5)?);
        event_name.push(r.get(6)?);
        universe_key.push(r.get(7)?);
        path.push(r.get(8)?);
        properties.push(r.get(9)?);
        duration_ms.push(r.get(10)?);
        ip_hash.push(r.get(11)?);
        ua_device.push(r.get(12)?);
        ua_browser.push(r.get(13)?);
        ua_os.push(r.get(14)?);
        country.push(r.get(15)?);
        city.push(r.get(16)?);
    }

    let cols: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(ids)),
        Arc::new(StringArray::from(timestamp)),
        Arc::new(StringArray::from(visitor_token)),
        Arc::new(StringArray::from(user_id)),
        Arc::new(StringArray::from(session_id)),
        Arc::new(StringArray::from(event_type)),
        Arc::new(StringArray::from(event_name)),
        Arc::new(StringArray::from(universe_key)),
        Arc::new(StringArray::from(path)),
        Arc::new(StringArray::from(properties)),
        Arc::new(Int64Array::from(duration_ms)),
        Arc::new(StringArray::from(ip_hash)),
        Arc::new(StringArray::from(ua_device)),
        Arc::new(StringArray::from(ua_browser)),
        Arc::new(StringArray::from(ua_os)),
        Arc::new(StringArray::from(country)),
        Arc::new(StringArray::from(city)),
    ];

    Ok(RecordBatch::try_new(parquet_schema(), cols)?)
}

/// Write a [`RecordBatch`] to `path` as zstd-compressed Parquet.
fn write_parquet(batch: &RecordBatch, path: &Path) -> anyhow::Result<()> {
    let props = WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .build();
    let file = std::fs::File::create(path)?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(props))?;
    writer.write(batch)?;
    writer.close()?;
    Ok(())
}

/// Number of rows recorded in a Parquet file's footer metadata.
fn parquet_row_count(path: &Path) -> anyhow::Result<i64> {
    let file = std::fs::File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    Ok(builder.metadata().file_metadata().num_rows())
}

/// Reclaim freed pages without a full-VACUUM rebuild. Best-effort: it is a no-op
/// until `auto_vacuum=INCREMENTAL` is effective (latent on a pre-existing DB until
/// a one-time full VACUUM — see the v087 migration / `docs/OPERATIONS.md`), so a
/// failure here must never abort the run (the rows are already safely archived).
pub fn reclaim_space(conn: &Connection) {
    if let Err(e) = conn.execute_batch("PRAGMA incremental_vacuum;") {
        tracing::debug!("CO-449: incremental_vacuum no-op/failed (non-fatal): {e}");
    }
}

/// Archive every cold month (oldest first), chunked so the first run on a tight
/// disk never materializes the whole table at once: each month is exported,
/// verified, deleted, and reclaimed before the next begins.
///
/// Returns an [`ArchiveReport`]. A per-month failure (export/verify mismatch) is
/// logged and that month is left untouched in `meta.db`; the run continues with
/// the remaining months rather than aborting wholesale.
pub fn archive_cold_telemetry(
    conn: &Connection,
    cfg: &ArchiveConfig,
) -> anyhow::Result<ArchiveReport> {
    let cutoff = cfg.now - Duration::days(cfg.hot_days);
    let cutoff_ym = (cutoff.year(), cutoff.month());

    let already = archived_months(conn)?;
    let mut report = ArchiveReport::default();

    for (year, month) in distinct_months(conn)? {
        // Only whole months strictly before the hot window's month are cold.
        if (year, month) >= cutoff_ym {
            continue;
        }
        if already.contains(&(year, month)) {
            report.skipped_existing.push((year, month));
            continue;
        }

        match archive_one_month(conn, cfg, year, month) {
            Ok(Some(m)) => {
                report.total_rows_archived += m.rows;
                report.archived.push(m);
            }
            Ok(None) => {} // empty month — nothing to do
            Err(e) => {
                tracing::error!(
                    year,
                    month,
                    "CO-449: archiving month failed; rows left intact in meta.db: {e}"
                );
            }
        }
    }

    Ok(report)
}

/// Archive a single month. The delete is the **last** mutating step and only runs
/// after the Parquet file exists and its row count matches SQLite, so a failure at
/// any earlier point leaves `telemetry_events` unchanged.
fn archive_one_month(
    conn: &Connection,
    cfg: &ArchiveConfig,
    year: i32,
    month: u32,
) -> anyhow::Result<Option<ArchivedMonth>> {
    let expected = month_row_count(conn, year, month)?;
    if expected == 0 {
        return Ok(None);
    }

    // 1. Export → temp file (one month per temp path, so no collision).
    let batch = month_record_batch(conn, year, month)?;
    std::fs::create_dir_all(&cfg.archive_dir)?;
    let tmp_path = cfg
        .archive_dir
        .join(format!(".part-{year:04}-{month:02}.tmp.parquet"));
    write_parquet(&batch, &tmp_path)?;

    // 2. Verify BEFORE deleting: Parquet row count == SQLite count, plus sha256.
    let parquet_rows = parquet_row_count(&tmp_path)?;
    if parquet_rows != expected {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::bail!(
            "verify failed for {year}-{month:02}: parquet {parquet_rows} != sqlite {expected}; \
             refusing to delete"
        );
    }
    let bytes_vec = std::fs::read(&tmp_path)?;
    let sha256 = to_hex(Sha256::digest(&bytes_vec).as_slice());
    let byte_len = bytes_vec.len() as u64;

    // 3. Move into the partitioned final path: year=YYYY/month=MM/part-<hash>.parquet
    let relative_path = format!(
        "year={year:04}/month={month:02}/part-{}.parquet",
        &sha256[..16]
    );
    let final_path = cfg.archive_dir.join(&relative_path);
    if let Some(parent) = final_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&tmp_path, &final_path)?;

    // 4. Delete the archived rows from meta.db (verified safe above).
    let deleted = conn.execute(
        "DELETE FROM telemetry_events \
         WHERE CAST(strftime('%Y', timestamp) AS INTEGER) = ?1 \
           AND CAST(strftime('%m', timestamp) AS INTEGER) = ?2",
        rusqlite::params![year, month],
    )? as i64;
    if deleted != expected {
        tracing::warn!(
            year,
            month,
            deleted,
            expected,
            "CO-449: deleted count differs from archived count (concurrent insert?)"
        );
    }

    // 5. Reclaim freed pages without a 2× full-VACUUM peak.
    reclaim_space(conn);

    // 6. Record the manifest row (traceability + dedupe on the next run).
    conn.execute(
        "INSERT OR IGNORE INTO telemetry_archives \
         (year, month, s3_key, rows, sha256, bytes, archived_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            year,
            month,
            relative_path,
            expected,
            sha256,
            byte_len as i64,
            cfg.now.to_rfc3339(),
        ],
    )?;

    tracing::info!(
        year,
        month,
        rows = expected,
        bytes = byte_len,
        "CO-449: archived telemetry month to Parquet"
    );

    Ok(Some(ArchivedMonth {
        year,
        month,
        relative_path,
        rows: expected,
        sha256,
        bytes: byte_len,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Build an in-memory DB with the `telemetry_events` + `telemetry_archives`
    /// schema (mirrors the real migrations) and `auto_vacuum=INCREMENTAL` enabled
    /// from creation so `incremental_vacuum` actually reclaims pages.
    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Must be set before any table is created to take effect without a VACUUM.
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")
            .unwrap();
        conn.execute_batch(
            "CREATE TABLE telemetry_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                visitor_token TEXT,
                user_id TEXT,
                session_id TEXT,
                event_type TEXT NOT NULL,
                event_name TEXT NOT NULL,
                universe_key TEXT,
                path TEXT,
                properties TEXT,
                duration_ms INTEGER,
                ip_hash TEXT,
                ua_device TEXT,
                ua_browser TEXT,
                ua_os TEXT,
                country TEXT,
                city TEXT
            );
            CREATE TABLE telemetry_archives (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                year INTEGER NOT NULL,
                month INTEGER NOT NULL,
                s3_key TEXT NOT NULL,
                rows INTEGER NOT NULL,
                sha256 TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                archived_at TEXT NOT NULL,
                UNIQUE(year, month)
            );",
        )
        .unwrap();
        conn
    }

    fn insert_event(conn: &Connection, timestamp: &str, name: &str) {
        conn.execute(
            "INSERT INTO telemetry_events \
             (timestamp, event_type, event_name, path, duration_ms, country) \
             VALUES (?1, 'pageview', ?2, '/home', 12, 'BR')",
            rusqlite::params![timestamp, name],
        )
        .unwrap();
    }

    fn cfg(dir: &Path) -> ArchiveConfig {
        ArchiveConfig {
            archive_dir: dir.to_path_buf(),
            hot_days: 90,
            // "now" = 2026-06-14 → cold cutoff 2026-03-16 → cold months < 2026-03.
            now: Utc.with_ymd_and_hms(2026, 6, 14, 0, 0, 0).unwrap(),
        }
    }

    fn total_rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM telemetry_events", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn archives_cold_months_and_keeps_hot_window() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db();

        // Cold: January + February 2026 (whole months < cutoff month 2026-03).
        for i in 0..5 {
            insert_event(&conn, &format!("2026-01-{:02}T10:00:00Z", i + 1), "jan");
        }
        for i in 0..3 {
            insert_event(&conn, &format!("2026-02-{:02}T10:00:00Z", i + 1), "feb");
        }
        // Hot: recent June rows stay (CO-360 dashboards read these).
        for i in 0..4 {
            insert_event(&conn, &format!("2026-06-{:02}T10:00:00Z", i + 1), "jun");
        }

        let report = archive_cold_telemetry(&conn, &cfg(dir.path())).unwrap();

        // Two cold months archived, oldest first; 8 rows moved out.
        assert_eq!(report.archived.len(), 2);
        assert_eq!(report.archived[0].year, 2026);
        assert_eq!(report.archived[0].month, 1);
        assert_eq!(report.archived[0].rows, 5);
        assert_eq!(report.archived[1].month, 2);
        assert_eq!(report.archived[1].rows, 3);
        assert_eq!(report.total_rows_archived, 8);

        // Hot June rows remain in meta.db — dashboards unaffected.
        assert_eq!(total_rows(&conn), 4);
        let jun: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM telemetry_events WHERE event_name = 'jun'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(jun, 4);

        // Partitioned Parquet files exist on disk.
        for m in &report.archived {
            assert!(dir.path().join(&m.relative_path).exists());
            assert!(m.relative_path.starts_with("year=2026/month="));
            assert!(m.relative_path.ends_with(".parquet"));
        }
    }

    #[test]
    fn manifest_records_each_archived_file() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db();
        insert_event(&conn, "2026-01-10T10:00:00Z", "a");
        insert_event(&conn, "2026-01-11T10:00:00Z", "b");

        archive_cold_telemetry(&conn, &cfg(dir.path())).unwrap();

        let (rows, sha, bytes, key): (i64, String, i64, String) = conn
            .query_row(
                "SELECT rows, sha256, bytes, s3_key FROM telemetry_archives \
                 WHERE year = 2026 AND month = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(rows, 2);
        assert_eq!(sha.len(), 64, "sha256 hex");
        assert!(bytes > 0);
        assert!(key.starts_with("year=2026/month=01/part-"));
    }

    #[test]
    fn zero_loss_total_equals_hot_plus_cold() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db();
        for i in 0..6 {
            insert_event(&conn, &format!("2026-01-{:02}T10:00:00Z", i + 1), "jan");
        }
        for i in 0..2 {
            insert_event(&conn, &format!("2026-06-{:02}T10:00:00Z", i + 1), "jun");
        }
        let original_total = total_rows(&conn);

        let report = archive_cold_telemetry(&conn, &cfg(dir.path())).unwrap();

        // Read the archived rows back from Parquet (the same path DuckDB uses).
        let mut cold_rows = 0i64;
        for m in &report.archived {
            cold_rows += parquet_row_count(&dir.path().join(&m.relative_path)).unwrap();
        }
        let hot_rows = total_rows(&conn);

        // Hot (SQLite) + cold (Parquet) == original — zero data lost.
        assert_eq!(hot_rows + cold_rows, original_total);
    }

    #[test]
    fn idempotent_second_run_skips_archived_months() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db();
        insert_event(&conn, "2026-01-10T10:00:00Z", "a");

        let first = archive_cold_telemetry(&conn, &cfg(dir.path())).unwrap();
        assert_eq!(first.archived.len(), 1);

        // A late-arriving event lands in an already-archived month.
        insert_event(&conn, "2026-01-20T10:00:00Z", "late");

        // Second run: the month is in the manifest → skipped, never re-archived
        // (no duplicate Parquet, no duplicate manifest row). The late row stays
        // safely in meta.db rather than being lost or double-written.
        let second = archive_cold_telemetry(&conn, &cfg(dir.path())).unwrap();
        assert_eq!(second.archived.len(), 0);
        assert_eq!(second.skipped_existing, vec![(2026, 1)]);
        assert_eq!(
            total_rows(&conn),
            1,
            "late row remains in meta.db, not lost"
        );

        // Still exactly one manifest row (no duplicate).
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM telemetry_archives", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn first_run_processes_months_chunked_oldest_first() {
        // Simulates the tight-disk first run: many months, processed one at a time
        // (export → verify → delete → reclaim), never all loaded at once.
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db();
        for (y, m) in [(2025, 10), (2025, 11), (2025, 12), (2026, 1), (2026, 2)] {
            for d in 1..=3 {
                insert_event(&conn, &format!("{y}-{m:02}-{d:02}T10:00:00Z"), "x");
            }
        }

        let report = archive_cold_telemetry(&conn, &cfg(dir.path())).unwrap();

        let order: Vec<(i32, u32)> = report.archived.iter().map(|m| (m.year, m.month)).collect();
        assert_eq!(
            order,
            vec![(2025, 10), (2025, 11), (2025, 12), (2026, 1), (2026, 2)],
            "months must be archived oldest-first (chunked)"
        );
        assert_eq!(report.total_rows_archived, 15);
        assert_eq!(total_rows(&conn), 0, "all cold rows removed from meta.db");
    }

    #[test]
    fn reclaim_frees_pages_without_full_vacuum() {
        // With auto_vacuum=INCREMENTAL, deleting + incremental_vacuum returns pages
        // to the OS (page_count drops) without a 2× rebuild peak.
        let conn = test_db();
        for i in 0..2000 {
            insert_event(
                &conn,
                &format!("2020-01-01T00:{:02}:{:02}Z", i / 60, i % 60),
                "bulk",
            );
        }
        let pages_before: i64 = conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap();

        conn.execute("DELETE FROM telemetry_events", []).unwrap();
        reclaim_space(&conn);

        let pages_after: i64 = conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap();
        assert!(
            pages_after < pages_before,
            "incremental_vacuum must shrink the DB ({pages_before} -> {pages_after})"
        );
    }

    #[test]
    fn cold_month_with_no_rows_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let conn = test_db();
        // Only hot data — nothing to archive.
        insert_event(&conn, "2026-06-10T10:00:00Z", "jun");
        let report = archive_cold_telemetry(&conn, &cfg(dir.path())).unwrap();
        assert!(report.archived.is_empty());
        assert_eq!(total_rows(&conn), 1);
    }
}
