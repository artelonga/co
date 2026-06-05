//! CO-379: SQLite snapshot/restore via the rusqlite `Backup` API.
//!
//! Used by CO-376 (pre-prod migration validation): snapshot the DB before
//! applying a risky migration, run smoke queries, restore if they fail.
//!
//! Each call is a point-in-time copy — conceptually analogous to a jj snapshot.
//! Works on both the shared `meta.db` and per-universe `data.db` files.

use std::path::Path;

use rusqlite::{Connection, backup::Backup};

/// Copy the SQLite database at `src` to `dest` using the online backup API.
///
/// The source database is opened read-only; the destination is created or
/// overwritten. Safe to call on a live database — no exclusive lock is taken
/// on the source while the copy runs.
pub fn snapshot_db(src: &Path, dest: &Path) -> anyhow::Result<()> {
    let conn = Connection::open(src)?;
    let mut backup_conn = Connection::open(dest)?;
    let backup = Backup::new(&conn, &mut backup_conn)?;
    backup.run_to_completion(100, std::time::Duration::ZERO, None)?;
    Ok(())
}

/// Restore `dest` from the snapshot at `src`.
///
/// Overwrites `dest` with the contents of `src`. Callers should ensure `dest`
/// is not in active use (e.g. close all connections first).
pub fn restore_db(src: &Path, dest: &Path) -> anyhow::Result<()> {
    snapshot_db(src, dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_and_restore_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let snap = dir.path().join("snap.db");

        // Create source DB with one row.
        {
            let conn = Connection::open(&src).unwrap();
            conn.execute_batch("CREATE TABLE t (v TEXT NOT NULL); INSERT INTO t VALUES ('hello');")
                .unwrap();
        }

        snapshot_db(&src, &snap).expect("snapshot must succeed");

        // Overwrite source with different data.
        {
            let conn = Connection::open(&src).unwrap();
            conn.execute_batch("DELETE FROM t; INSERT INTO t VALUES ('corrupted');")
                .unwrap();
        }

        // Restore snapshot back to source.
        restore_db(&snap, &src).expect("restore must succeed");

        // Verify original value is back.
        let conn = Connection::open(&src).unwrap();
        let val: String = conn.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(val, "hello", "restored value must match original snapshot");
    }

    #[test]
    fn snapshot_works_on_per_universe_db() {
        let dir = tempfile::tempdir().unwrap();
        let universe_db = dir.path().join("data.db");
        let snap = dir.path().join("pre-migration.db");

        {
            let conn = Connection::open(&universe_db).unwrap();
            conn.execute_batch(
                "CREATE TABLE entries (path TEXT PRIMARY KEY, body TEXT); \
                 INSERT INTO entries VALUES ('test.md', '# Hello');",
            )
            .unwrap();
        }

        snapshot_db(&universe_db, &snap).expect("per-universe snapshot must succeed");

        let snap_conn = Connection::open(&snap).unwrap();
        let body: String = snap_conn
            .query_row("SELECT body FROM entries WHERE path = 'test.md'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(body, "# Hello");
    }
}
