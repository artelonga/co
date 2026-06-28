//! CO-496 (tier-3 scalability): a concurrent **read** pool over `meta.db`.
//!
//! Every authenticated request resolves an API token → user, and several routes
//! then read user / ownership rows from `meta.db`. Until now all of those reads
//! went through the single global `Mutex<Storage>` write lock (CoreState.storage),
//! so the auth-lookup path serialized the entire process at a low-thousands req/s
//! ceiling — even though they are pure, side-effect-free `SELECT`s.
//!
//! `meta.db` runs in WAL mode (set in [`Storage::new`]), which permits many
//! concurrent readers alongside a single writer. This pool exploits that: it
//! holds a small set of **read-only** connections (`query_only=ON`,
//! `SQLITE_OPEN_READ_ONLY`) and hands one out per lookup, so N auth lookups run
//! truly in parallel and never contend on the writer's mutex.
//!
//! Design choice (vs. r2d2): a hand-rolled checkout stack keeps the dependency
//! surface unchanged and is trivially testable. The acceptance criterion
//! explicitly allows "an r2d2/rusqlite read pool (or `RwLock` Storage)" — this is
//! the rusqlite variant. Connections are pre-opened up to `capacity`; if the pool
//! is momentarily drained under a burst, a transient connection is opened on the
//! spot and dropped on return (so a burst degrades to "open a connection" rather
//! than "block on the write lock"), keeping the path lock-free.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags, params};

use crate::storage::schema::parse_datetime;

/// Default number of pooled read-only connections. Override with
/// `CO_META_READ_POOL_SIZE`. WAL readers are cheap, but each holds an fd, so the
/// default is modest; raise it for a high-concurrency deployment.
pub const DEFAULT_READ_POOL_SIZE: usize = 8;

/// Parse the read-pool size from an env value, clamping to `[1, 256]` and falling
/// back to [`DEFAULT_READ_POOL_SIZE`] when unset/empty/invalid/zero. Pure →
/// unit-testable.
pub fn parse_pool_size(raw: Option<String>) -> usize {
    raw.and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .map(|n| n.min(256))
        .unwrap_or(DEFAULT_READ_POOL_SIZE)
}

/// A pool of read-only SQLite connections to `meta.db`.
///
/// Clone-free: wrap in `Arc` (the field on [`Storage`] and the clone on
/// `CoreState` share one pool). Read lookups go through [`MetaReadPool::with_conn`]
/// or the typed helpers; none of them ever takes the global write mutex.
pub struct MetaReadPool {
    db_path: PathBuf,
    idle: Mutex<Vec<Connection>>,
    capacity: usize,
}

impl MetaReadPool {
    /// Open `capacity` read-only connections to the `meta.db` under `data_dir`.
    ///
    /// MUST be called *after* the schema exists (i.e. after migrations), since a
    /// read-only connection cannot create the file. [`Storage::new`] enforces this
    /// ordering. A per-connection open failure is logged and skipped rather than
    /// fatal — a smaller-than-requested pool still works (it just falls back to
    /// transient connections sooner).
    pub fn open(data_dir: impl AsRef<Path>, capacity: usize) -> Self {
        let db_path = data_dir.as_ref().join("meta.db");
        let capacity = capacity.max(1);
        let mut idle = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            match Self::open_read_conn(&db_path) {
                Ok(conn) => idle.push(conn),
                Err(e) => {
                    tracing::warn!(error = %e, "CO-496: meta read-pool connection open failed (degrading pool size)");
                    break;
                }
            }
        }
        Self {
            db_path,
            idle: Mutex::new(idle),
            capacity,
        }
    }

    /// Open one read-only connection: `SQLITE_OPEN_READ_ONLY` + `query_only=ON`
    /// (defense in depth — a read connection can never mutate, even via a stray
    /// `INSERT`). WAL readers do not block the writer.
    fn open_read_conn(db_path: &Path) -> rusqlite::Result<Connection> {
        let conn = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA query_only=ON; PRAGMA foreign_keys=ON;")?;
        Ok(conn)
    }

    /// Run `f` against a pooled read-only connection, returning it to the pool
    /// afterward. If the pool is drained (burst), a transient connection is opened
    /// and dropped on return rather than blocking — so the read path is always
    /// lock-free with respect to the global write mutex.
    pub fn with_conn<F, R>(&self, f: F) -> rusqlite::Result<R>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<R>,
    {
        let conn = {
            let mut idle = self.idle.lock();
            idle.pop()
        };
        let (conn, pooled) = match conn {
            Some(c) => (c, true),
            None => (Self::open_read_conn(&self.db_path)?, false),
        };
        let result = f(&conn);
        if pooled {
            let mut idle = self.idle.lock();
            if idle.len() < self.capacity {
                idle.push(conn);
            }
        }
        result
    }

    /// Number of connections currently idle in the pool (test/observability).
    pub fn idle_count(&self) -> usize {
        self.idle.lock().len()
    }

    // -----------------------------------------------------------------------
    // Typed auth-lookup helpers — the hot, side-effect-free reads CO-496 moves
    // off the global write mutex. Each mirrors the equivalent method on
    // `Storage` exactly, but runs on a concurrent read connection.
    // -----------------------------------------------------------------------

    /// Concurrent read of a user by id (mirrors [`Storage::get_user_by_id`]).
    /// Returns `None` on a miss or any read error (callers treat both as
    /// "no such user", matching the `Storage` variant's `.ok()` semantics).
    pub fn get_user_by_id(&self, id: &str) -> Option<crate::models::User> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, COALESCE(email,'') as email, display_name, tier, created_at \
                 FROM users WHERE id = ?1",
                params![id],
                |row| {
                    Ok(crate::models::User {
                        id: row.get(0)?,
                        email: row.get(1)?,
                        display_name: row.get(2)?,
                        tier: row.get(3)?,
                        created_at: parse_datetime(&row.get::<_, String>(4)?),
                        usuario: None,
                    })
                },
            )
        })
        .ok()
    }

    /// Concurrent read of the user id owning a WhatsApp number (mirrors
    /// [`Storage::get_user_id_by_whatsapp`]). Backed by `idx_users_whatsapp`
    /// (CO-496 migration v095).
    pub fn get_user_id_by_whatsapp(&self, whatsapp: &str) -> Option<String> {
        let normalized = whatsapp.trim();
        if normalized.is_empty() {
            return None;
        }
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id FROM users WHERE whatsapp = ?1",
                params![normalized],
                |row| row.get::<_, String>(0),
            )
        })
        .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    #[test]
    fn parse_pool_size_defaults_and_clamps() {
        assert_eq!(parse_pool_size(None), DEFAULT_READ_POOL_SIZE);
        assert_eq!(parse_pool_size(Some("16".into())), 16);
        assert_eq!(parse_pool_size(Some("  4 ".into())), 4);
        // Invalid / empty / zero → default (never a 0-connection pool).
        assert_eq!(parse_pool_size(Some(String::new())), DEFAULT_READ_POOL_SIZE);
        assert_eq!(parse_pool_size(Some("abc".into())), DEFAULT_READ_POOL_SIZE);
        assert_eq!(parse_pool_size(Some("0".into())), DEFAULT_READ_POOL_SIZE);
        // Over-large is clamped.
        assert_eq!(parse_pool_size(Some("100000".into())), 256);
    }

    #[test]
    fn pool_reads_user_written_through_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        let alice = storage.create_user("alice@example.com", "Alice").unwrap();

        let pool = MetaReadPool::open(dir.path(), 4);
        // The read pool sees the committed row (WAL: readers see the last commit).
        let read = pool.get_user_by_id(&alice.id).expect("read alice");
        assert_eq!(read.id, alice.id);
        assert_eq!(read.email, "alice@example.com");
        // A miss returns None, not an error.
        assert!(pool.get_user_by_id("nobody").is_none());
    }

    #[test]
    fn pool_resolves_whatsapp_uniqueness() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = Storage::new(dir.path());
        let alice = storage.create_user("a@b.c", "Alice").unwrap();
        storage
            .set_user_whatsapp(&alice.id, "5541999990000")
            .unwrap();

        let pool = MetaReadPool::open(dir.path(), 2);
        assert_eq!(
            pool.get_user_id_by_whatsapp("5541999990000").as_deref(),
            Some(alice.id.as_str())
        );
        assert!(pool.get_user_id_by_whatsapp("0000").is_none());
        assert!(pool.get_user_id_by_whatsapp("   ").is_none());
    }

    #[test]
    fn checkout_returns_connection_to_pool() {
        let dir = tempfile::tempdir().unwrap();
        let _storage = Storage::new(dir.path());
        let pool = MetaReadPool::open(dir.path(), 3);
        assert_eq!(pool.idle_count(), 3);
        pool.with_conn(|c| c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0)))
            .unwrap();
        // Borrowed then returned → still 3 idle.
        assert_eq!(pool.idle_count(), 3);
    }

    #[test]
    fn read_only_connection_rejects_writes() {
        let dir = tempfile::tempdir().unwrap();
        let _storage = Storage::new(dir.path());
        let pool = MetaReadPool::open(dir.path(), 1);
        // query_only=ON + SQLITE_OPEN_READ_ONLY → a write must fail, never corrupt.
        let r = pool.with_conn(|c| {
            c.execute(
                "INSERT INTO users (id, email, created_at) VALUES ('x','x','x')",
                [],
            )
        });
        assert!(r.is_err(), "read pool must refuse writes");
    }
}
