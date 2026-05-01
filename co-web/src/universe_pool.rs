//! Per-universe SQLite connection pool with LRU eviction.
//!
//! Each universe keeps its entries in its own `data.db` file under a 2-level
//! filesystem fanout derived from an xxHash of the universe key:
//!
//! ```text
//! {data_dir}/universes/{ab}/{cd}/{universe-key}/data.db
//! ```
//!
//! The pool caps open connections at `capacity` (default 1000). When the cache
//! is full, the least-recently-used connection is closed before a new one is
//! opened.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lru::LruCache;
use rusqlite::Connection;

// ---------------------------------------------------------------------------
// Schema for per-universe data.db
// ---------------------------------------------------------------------------

const UNIVERSE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
    path              TEXT NOT NULL,
    universe_key      TEXT NOT NULL,
    entry_type        TEXT NOT NULL,
    title             TEXT,
    frontmatter_json  TEXT NOT NULL DEFAULT '{}',
    payload           TEXT NOT NULL DEFAULT '{}',
    body              TEXT NOT NULL DEFAULT '',
    body_hash         TEXT NOT NULL DEFAULT '',
    created_at        TEXT,
    updated_at        TEXT,
    PRIMARY KEY (universe_key, path)
);
CREATE INDEX IF NOT EXISTS idx_entries_type    ON entries(universe_key, entry_type);
CREATE INDEX IF NOT EXISTS idx_entries_updated ON entries(universe_key, updated_at);

CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    universe_key UNINDEXED,
    path         UNINDEXED,
    title,
    body
);

CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
";

// ---------------------------------------------------------------------------
// UniversePool
// ---------------------------------------------------------------------------

struct PoolInner {
    cache: LruCache<String, Arc<Mutex<Connection>>>,
}

/// Connection pool for per-universe SQLite databases.
pub struct UniversePool {
    data_dir: PathBuf,
    inner: Mutex<PoolInner>,
}

impl UniversePool {
    /// Create a new pool.
    ///
    /// `data_dir` is the root data directory (same as `Storage::data_dir`).
    /// `capacity` is the maximum number of simultaneously open connections.
    pub fn new(data_dir: impl AsRef<Path>, capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).expect("capacity >= 1");
        Self {
            data_dir: data_dir.as_ref().to_path_buf(),
            inner: Mutex::new(PoolInner {
                cache: LruCache::new(cap),
            }),
        }
    }

    // -------------------------------------------------------------------------
    // Path helpers
    // -------------------------------------------------------------------------

    /// 2-level directory prefix derived from an xxHash of the universe key.
    ///
    /// The hash is split into two single-byte hex pairs to give 256×256 = 65 536
    /// possible level-1/level-2 directory combinations, keeping each directory
    /// small even at millions of universes.
    fn hash_prefix(key: &str) -> (String, String) {
        let h = xxhash_rust::xxh3::xxh3_64(key.as_bytes());
        // Take byte 7 (most significant) and byte 6 as the two prefix levels.
        let b7 = ((h >> 56) & 0xff) as u8;
        let b6 = ((h >> 48) & 0xff) as u8;
        (format!("{b7:02x}"), format!("{b6:02x}"))
    }

    /// Directory that holds `data.db` for a given universe key.
    pub fn universe_dir(&self, key: &str) -> PathBuf {
        let (l1, l2) = Self::hash_prefix(key);
        self.data_dir.join("universes").join(l1).join(l2).join(key)
    }

    /// Full path to the `data.db` file for a given universe key.
    pub fn db_path(&self, key: &str) -> PathBuf {
        self.universe_dir(key).join("data.db")
    }

    // -------------------------------------------------------------------------
    // Connection management
    // -------------------------------------------------------------------------

    /// Get (or open) the connection for a universe, returning an `Arc<Mutex<Connection>>`.
    ///
    /// Opens and migrates the database on first access. Evicts the LRU connection
    /// when the cache is full.
    pub fn get_or_open(&self, key: &str) -> Arc<Mutex<Connection>> {
        let mut inner = self.inner.lock().expect("pool lock");

        if let Some(arc) = inner.cache.get(key) {
            return arc.clone();
        }

        // Open a new connection
        let db_path = self.db_path(key);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).expect("create universe dir");
        }

        let conn = Connection::open(&db_path).expect("open universe data.db");
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .expect("universe db pragmas");
        run_universe_migrations(&conn);

        let arc = Arc::new(Mutex::new(conn));
        inner.cache.put(key.to_string(), arc.clone());
        arc
    }

    /// Evict a universe's connection from the pool (closes the connection if no
    /// other holders exist).
    pub fn evict(&self, key: &str) {
        let mut inner = self.inner.lock().expect("pool lock");
        inner.cache.pop(key);
    }

    /// Number of currently open connections in the pool.
    pub fn open_count(&self) -> usize {
        self.inner.lock().expect("pool lock").cache.len()
    }
}

// ---------------------------------------------------------------------------
// Universe-level migrations
// ---------------------------------------------------------------------------

fn run_universe_migrations(conn: &Connection) {
    conn.execute_batch(UNIVERSE_SCHEMA)
        .expect("universe schema migration");

    let v: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if v < 1 {
        conn.execute("INSERT INTO schema_version (version) VALUES (1)", [])
            .expect("universe schema_version v1");
    }
}
