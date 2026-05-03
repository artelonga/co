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

/// Exposed for in-crate tests that need to open an in-memory database with the
/// full per-universe schema applied.
#[cfg(test)]
pub(crate) const UNIVERSE_SCHEMA_FOR_TEST: &str = UNIVERSE_SCHEMA;

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

-- CO-73: temporal model — indexed semantic dates per entry
CREATE TABLE IF NOT EXISTS entry_dates (
    universe_key TEXT NOT NULL,
    entry_path   TEXT NOT NULL,
    semantic     TEXT NOT NULL,
    value        TEXT NOT NULL,
    PRIMARY KEY (universe_key, entry_path, semantic)
);
CREATE INDEX IF NOT EXISTS idx_entry_dates_range ON entry_dates(universe_key, semantic, value);

CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);

-- CO-74: typed FK relationship graph — one row per directed edge
CREATE TABLE IF NOT EXISTS entry_relations (
    universe_key  TEXT NOT NULL,
    from_path     TEXT NOT NULL,
    to_path       TEXT NOT NULL,
    relation_type TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    PRIMARY KEY (universe_key, from_path, to_path, relation_type)
);
CREATE INDEX IF NOT EXISTS idx_er_from
    ON entry_relations(universe_key, from_path, relation_type);
CREATE INDEX IF NOT EXISTS idx_er_to
    ON entry_relations(universe_key, to_path,   relation_type);

-- CO-146: content-addressable binary assets (Phase 1 of CO-145).
-- Bytes live on disk at universe_dir/blobs/<aa>/<bb>/<sha256>.
-- Phase 1 stores plaintext; CO-148 will add ciphertext + nonce columns.
CREATE TABLE IF NOT EXISTS assets (
    sha256        TEXT PRIMARY KEY,
    blob_path     TEXT NOT NULL,
    mime          TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL,
    filename      TEXT,
    created_at_ns INTEGER NOT NULL,
    created_by    TEXT,
    refcount      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_assets_mime       ON assets(mime);
CREATE INDEX IF NOT EXISTS idx_assets_created_at ON assets(created_at_ns);

-- CO-147: per-asset user-supplied tags (Phase 2 of CO-145).
CREATE TABLE IF NOT EXISTS asset_tags (
    sha256 TEXT NOT NULL,
    tag    TEXT NOT NULL,
    PRIMARY KEY (sha256, tag),
    FOREIGN KEY (sha256) REFERENCES assets(sha256) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_asset_tags_tag ON asset_tags(tag);

-- CO-147: searchable plaintext shadow of frontmatter (Phase 2 of CO-145).
-- Survives encryption-at-rest in Phase 3 (CO-148) because body bytes get
-- encrypted but the typed metadata that drives search/sort/filter stays here.
CREATE TABLE IF NOT EXISTS frontmatter_index (
    entry_path    TEXT PRIMARY KEY,
    title         TEXT,
    entry_type    TEXT,
    status        TEXT,
    tags_json     TEXT,
    created_at_ns INTEGER,
    due_at_ns     INTEGER,
    parent_path   TEXT
);
CREATE INDEX IF NOT EXISTS idx_fm_type   ON frontmatter_index(entry_type);
CREATE INDEX IF NOT EXISTS idx_fm_status ON frontmatter_index(status);

-- CO-156: `reference` content type shadow table.
-- One row per `.md` card with entry_type = 'reference'.
-- Populated on every reference-card write; blob_sha256 is resolved on index.
CREATE TABLE IF NOT EXISTS references_meta (
    universe_key  TEXT NOT NULL,
    entry_path    TEXT NOT NULL,
    file          TEXT,
    blob_sha256   TEXT,
    url           TEXT,
    medium        TEXT NOT NULL DEFAULT 'unknown',
    mime          TEXT,
    size_bytes    INTEGER,
    language      TEXT,
    seed_status   TEXT NOT NULL DEFAULT 'stub',
    indexed_at    TEXT NOT NULL,
    PRIMARY KEY (universe_key, entry_path)
);
CREATE INDEX IF NOT EXISTS idx_refs_medium      ON references_meta(medium);
CREATE INDEX IF NOT EXISTS idx_refs_seed_status ON references_meta(seed_status);
CREATE INDEX IF NOT EXISTS idx_refs_blob
    ON references_meta(blob_sha256) WHERE blob_sha256 IS NOT NULL;

-- CO-156: FTS index over reference cards — title, body, and transcription.
CREATE VIRTUAL TABLE IF NOT EXISTS references_fts USING fts5(
    universe_key UNINDEXED,
    entry_path   UNINDEXED,
    title,
    body,
    transcription
);
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
    if v < 2 {
        // CO-73: entry_dates table already created via UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (2)", [])
            .expect("universe schema_version v2");
    }
    if v < 3 {
        // CO-74: entry_relations table already created via UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (3)", [])
            .expect("universe schema_version v3");
    }
    if v < 4 {
        // CO-146: assets table already created via UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (4)", [])
            .expect("universe schema_version v4");
    }
    if v < 5 {
        // CO-147: asset_tags + frontmatter_index already created via
        // UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (5)", [])
            .expect("universe schema_version v5");
    }
    if v < 6 {
        // CO-148: encryption envelope columns on assets.
        // `encrypted = 0` rows are Phase 1 plaintext blobs (stay readable).
        // `encrypted = 1` rows are ChaCha20-Poly1305 ciphertext with `nonce`
        // populated and `cipher_size` reflecting the on-disk byte count.
        ensure_universe_column(conn, "assets", "nonce", "BLOB");
        ensure_universe_column(conn, "assets", "cipher_size", "INTEGER");
        ensure_universe_column(conn, "assets", "encrypted", "INTEGER NOT NULL DEFAULT 0");
        conn.execute("INSERT INTO schema_version (version) VALUES (6)", [])
            .expect("universe schema_version v6");
    }
    if v < 7 {
        // CO-156: references_meta + references_fts already created via UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (7)", [])
            .expect("universe schema_version v7");
    }
}

#[cfg(test)]
pub(crate) fn run_universe_migrations_for_test(conn: &Connection) {
    run_universe_migrations(conn);
}

/// Per-universe DB version of the storage.rs `ensure_column` helper. Mirrors
/// the CO-137 drift-safe pattern.
fn ensure_universe_column(conn: &Connection, table: &str, column: &str, def: &str) {
    let exists: bool = conn
        .query_row(
            &format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"),
            rusqlite::params![column],
            |_| Ok(true),
        )
        .ok()
        .unwrap_or(false);
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {def};"))
            .unwrap_or_else(|e| panic!("ensure_universe_column {table}.{column}: {e}"));
    }
}
