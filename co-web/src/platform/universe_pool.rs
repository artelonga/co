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

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use lru::LruCache;
use rusqlite::Connection;

// ---------------------------------------------------------------------------
// PoolError — CO-406: a per-universe open failure that MUST NOT crash the
// process. A single universe whose DB cannot be opened/migrated (disk full,
// I/O error, corrupt -shm) becomes unavailable; every other universe keeps
// serving. Callers on the request path translate this into a 503.
// ---------------------------------------------------------------------------

/// Why a single universe could not be brought online. Carrying the reason
/// string lets the route layer surface a useful 503 body and lets `atividades`
/// record the failure.
#[derive(Debug, Clone)]
pub struct PoolError {
    /// The universe key that failed to open.
    pub universe: String,
    /// Which step failed (`mkdir` / `open` / `pragmas` / `migrations`).
    pub stage: &'static str,
    /// The underlying error rendered as a string (sqlite/io message).
    pub reason: String,
}

impl PoolError {
    fn new(universe: &str, stage: &'static str, reason: impl std::fmt::Display) -> Self {
        Self {
            universe: universe.to_string(),
            stage,
            reason: reason.to_string(),
        }
    }
}

impl std::fmt::Display for PoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "universe '{}' unavailable ({}): {}",
            self.universe, self.stage, self.reason
        )
    }
}

impl std::error::Error for PoolError {}

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
    body_lines        INTEGER NOT NULL DEFAULT 0,
    body_words        INTEGER NOT NULL DEFAULT 0,
    body_chars        INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT,
    updated_at        TEXT,
    entry_origin      TEXT NOT NULL DEFAULT '',
    review_status     TEXT NOT NULL DEFAULT 'published',
    submitted_by      TEXT,
    submitted_at      TEXT,
    reviewed_by       TEXT,
    reviewed_at       TEXT,
    event_at_ms       INTEGER,
    due_at_ms         INTEGER,
    scheduled_at_ms   INTEGER,
    PRIMARY KEY (universe_key, path)
);
CREATE INDEX IF NOT EXISTS idx_entries_type    ON entries(universe_key, entry_type);
CREATE INDEX IF NOT EXISTS idx_entries_updated ON entries(universe_key, updated_at);
-- CO-330: supports anon published-only filter (json_extract over frontmatter_json).
CREATE INDEX IF NOT EXISTS idx_entries_published
    ON entries(universe_key, json_extract(frontmatter_json, '$.published'));
-- CO-354: the partial review_status index is created by migration v18, NOT
-- here. The base batch runs on existing DBs whose entries table predates the
-- column (added in v18); an index referencing it here panics the server at
-- boot before migrations can run (staging crash, 2026-06-10).
-- CO-387: idx_entries_event_ms lives in migration v19 for the same reason —
-- the ms columns are migration-added on existing DBs.

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
-- CO-153: to_universe added for cross-universe refs (NULL = same universe)
-- CO-363: link_text stores optional alias label from [[target|label]] wikilinks
CREATE TABLE IF NOT EXISTS entry_relations (
    universe_key  TEXT NOT NULL,
    from_path     TEXT NOT NULL,
    to_path       TEXT NOT NULL,
    relation_type TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    to_universe   TEXT,
    valid_from    TEXT,
    valid_until   TEXT,
    link_text     TEXT,
    PRIMARY KEY (universe_key, from_path, to_path, relation_type)
);
CREATE INDEX IF NOT EXISTS idx_er_from
    ON entry_relations(universe_key, from_path, relation_type);
CREATE INDEX IF NOT EXISTS idx_er_to
    ON entry_relations(universe_key, to_path,   relation_type);
-- idx_er_to_universe lives in the v7 migration block (universe_pool.rs):
-- can't be created here because existing prod DBs predating v7 don't have
-- the `to_universe` column, and CREATE TABLE IF NOT EXISTS skips the
-- table creation, so the column isn't there when this index runs.

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

-- CO-154: per-citation index. One row per item in frontmatter.references[]
-- of any entry; excerpt_body is the matching ## Referência: section from
-- the entry body. Distinct from references_meta (CO-156) which indexes
-- the reference *card* itself: this indexes citations *within* any entry.
CREATE TABLE IF NOT EXISTS references_index (
    universe_key  TEXT NOT NULL,
    entry_path    TEXT NOT NULL,
    ref_index     INTEGER NOT NULL,
    url           TEXT,
    source        TEXT NOT NULL,
    retrieved     TEXT,
    excerpt_body  TEXT,
    PRIMARY KEY (universe_key, entry_path, ref_index)
);
CREATE INDEX IF NOT EXISTS idx_refs_source
    ON references_index(source);
CREATE INDEX IF NOT EXISTS idx_refs_url
    ON references_index(url) WHERE url IS NOT NULL;

-- CO-154: FTS over source + excerpt for full-text search across reference
-- excerpts (per-citation index).
CREATE VIRTUAL TABLE IF NOT EXISTS references_fts USING fts5(
    universe_key UNINDEXED,
    entry_path   UNINDEXED,
    ref_index    UNINDEXED,
    source,
    excerpt_body
);

-- CO-156 + CO-158: reference content type shadow table.
-- One row per edition of a .md card with entry_type = reference.
-- work_id groups all editions of the same conceptual work.
-- edition_id identifies a concrete artifact (e.g. usp-1959, default).
-- primary_layer = min(layer) in primary_source_chain; null if not authored.
CREATE TABLE IF NOT EXISTS references_meta (
    universe_key  TEXT NOT NULL,
    entry_path    TEXT NOT NULL,
    edition_id    TEXT NOT NULL DEFAULT 'default',
    work_id       TEXT NOT NULL DEFAULT '',
    primary_layer INTEGER,
    file          TEXT,
    blob_sha256   TEXT,
    url           TEXT,
    medium        TEXT NOT NULL DEFAULT 'unknown',
    mime          TEXT,
    size_bytes    INTEGER,
    language      TEXT,
    seed_status   TEXT NOT NULL DEFAULT 'stub',
    indexed_at    TEXT NOT NULL,
    PRIMARY KEY (universe_key, entry_path, edition_id)
);
CREATE INDEX IF NOT EXISTS idx_refs_medium        ON references_meta(medium);
CREATE INDEX IF NOT EXISTS idx_refs_seed_status   ON references_meta(seed_status);
CREATE INDEX IF NOT EXISTS idx_refs_blob
    ON references_meta(blob_sha256) WHERE blob_sha256 IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_refs_work_id       ON references_meta(work_id);
CREATE INDEX IF NOT EXISTS idx_refs_primary_layer
    ON references_meta(primary_layer) WHERE primary_layer IS NOT NULL;

-- CO-156: FTS index over reference cards — title, body, and transcription.
-- Renamed from `references_fts` to avoid collision with CO-154's per-citation
-- FTS above (different content, different shape, both legitimately needed).
CREATE VIRTUAL TABLE IF NOT EXISTS reference_cards_fts USING fts5(
    universe_key UNINDEXED,
    entry_path   UNINDEXED,
    title,
    body,
    transcription
);

-- CO-164: vector embedding store for semantic search.
-- 384 × f32 = 1536 bytes per embedding (all-MiniLM-L6-v2).
-- body_hash lets the worker skip re-embedding unchanged content.
CREATE TABLE IF NOT EXISTS entry_embeddings (
    universe_key TEXT NOT NULL,
    path         TEXT NOT NULL,
    body_hash    TEXT NOT NULL,
    embedding    BLOB NOT NULL,
    model        TEXT NOT NULL,
    indexed_at   TEXT NOT NULL,
    PRIMARY KEY (universe_key, path)
);
CREATE INDEX IF NOT EXISTS idx_emb_body_hash ON entry_embeddings(body_hash);
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
    /// CO-406: universes whose last open attempt failed, with the reason.
    /// Advisory only — entries are cleared on a successful (re)open and a fresh
    /// attempt is always made on the next access (lazy retry), so a transient
    /// environment failure (disk full, I/O error) self-heals without a restart.
    unavailable: Mutex<HashMap<String, PoolError>>,
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
            unavailable: Mutex::new(HashMap::new()),
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

    /// CO-406: get (or open) the connection for a universe **without panicking**.
    ///
    /// Opens and migrates the database on first access, evicting the LRU
    /// connection when the cache is full. On any environment failure (mkdir,
    /// open, pragmas, migrations) it records the universe as unavailable and
    /// returns `Err(PoolError)` instead of panicking — so one bad universe can
    /// never take the whole process down (2026-06-11 outage).
    ///
    /// The failure is *not* cached as a connection: the next call re-attempts
    /// the open, so the universe recovers automatically once the environment
    /// failure clears (disk freed, volume extended) — no machine restart.
    pub fn try_get_or_open(&self, key: &str) -> Result<Arc<Mutex<Connection>>, PoolError> {
        {
            let mut inner = self.inner.lock().expect("pool lock");
            if let Some(arc) = inner.cache.get(key) {
                return Ok(arc.clone());
            }
        }

        match self.build_connection(key) {
            Ok(conn) => {
                let arc = Arc::new(Mutex::new(conn));
                {
                    let mut inner = self.inner.lock().expect("pool lock");
                    inner.cache.put(key.to_string(), arc.clone());
                }
                // Recovered (or first success): clear any stale unavailable mark.
                self.unavailable
                    .lock()
                    .expect("unavailable lock")
                    .remove(key);
                Ok(arc)
            }
            Err(e) => {
                tracing::error!(
                    universe = %key,
                    stage = e.stage,
                    reason = %e.reason,
                    "CO-406: universe pool open failed — marking unavailable, other universes unaffected"
                );
                self.unavailable
                    .lock()
                    .expect("unavailable lock")
                    .insert(key.to_string(), e.clone());
                Err(e)
            }
        }
    }

    /// Build a fresh connection for `key` (mkdir + open + pragmas + migrations),
    /// returning a `PoolError` tagged with the failing stage instead of
    /// panicking. Holds no pool lock.
    fn build_connection(&self, key: &str) -> Result<Connection, PoolError> {
        let db_path = self.db_path(key);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| PoolError::new(key, "mkdir", e))?;
        }
        let conn = Connection::open(&db_path).map_err(|e| PoolError::new(key, "open", e))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| PoolError::new(key, "pragmas", e))?;
        run_universe_migrations(&conn, key).map_err(|e| PoolError::new(key, "migrations", e))?;
        Ok(conn)
    }

    /// Get (or open) the connection for a universe, returning an `Arc<Mutex<Connection>>`.
    ///
    /// Panicking convenience wrapper around [`try_get_or_open`] for internal
    /// infrastructure call sites (seeding, recompute, clone/import) where a
    /// failure is a genuine bug rather than an isolatable per-universe outage.
    /// **Request-path code must use [`try_get_or_open`]** so a single bad
    /// universe degrades to a 503 instead of aborting the task.
    ///
    /// [`try_get_or_open`]: Self::try_get_or_open
    pub fn get_or_open(&self, key: &str) -> Arc<Mutex<Connection>> {
        self.try_get_or_open(key)
            .unwrap_or_else(|e| panic!("universe pool open failed: {e}"))
    }

    /// Evict a universe's connection from the pool (closes the connection if no
    /// other holders exist).
    pub fn evict(&self, key: &str) {
        let mut inner = self.inner.lock().expect("pool lock");
        inner.cache.pop(key);
    }

    /// CO-406: admin reopen — drop any cached connection and forget the
    /// unavailable mark, then attempt a fresh open. Lets an operator recover a
    /// universe after fixing the environment without restarting the machine.
    pub fn reopen(&self, key: &str) -> Result<(), PoolError> {
        self.evict(key);
        self.unavailable
            .lock()
            .expect("unavailable lock")
            .remove(key);
        self.try_get_or_open(key).map(|_| ())
    }

    /// CO-406: the recorded reason a universe is currently marked unavailable,
    /// if any. Cleared on the next successful (re)open.
    pub fn unavailable_reason(&self, key: &str) -> Option<PoolError> {
        self.unavailable
            .lock()
            .expect("unavailable lock")
            .get(key)
            .cloned()
    }

    /// CO-406: snapshot of all universes currently marked unavailable
    /// (key → reason), for the admin/gestão surface.
    pub fn unavailable_universes(&self) -> Vec<PoolError> {
        self.unavailable
            .lock()
            .expect("unavailable lock")
            .values()
            .cloned()
            .collect()
    }

    /// Number of currently open connections in the pool.
    pub fn open_count(&self) -> usize {
        self.inner.lock().expect("pool lock").cache.len()
    }

    /// CO-433: open a per-universe [`EntryStore`] — the pool is the factory over
    /// the storage backend. SQLite ([`SqliteEntryStore`]) is the default impl
    /// behind the trait; a future S3/Postgres backend plugs in here without
    /// touching handlers. Returns `Err(PoolError)` on a per-universe open
    /// failure (never panics), mirroring [`try_get_or_open`].
    ///
    /// Because each universe's store wraps that universe's own
    /// `Arc<Mutex<Connection>>`, content writes to distinct universes never
    /// serialize against each other or against the global `Mutex<Storage>`.
    ///
    /// [`EntryStore`]: crate::repository::EntryStore
    /// [`SqliteEntryStore`]: crate::repository::SqliteEntryStore
    /// [`try_get_or_open`]: Self::try_get_or_open
    pub fn entry_store(
        &self,
        key: &str,
    ) -> Result<Arc<dyn crate::repository::EntryStore>, PoolError> {
        let conn = self.try_get_or_open(key)?;
        Ok(Arc::new(crate::repository::SqliteEntryStore::new(key, conn)))
    }
}

// ---------------------------------------------------------------------------
// Universe-level migrations
// ---------------------------------------------------------------------------

/// Apply the per-universe schema + incremental migrations.
///
/// CO-406: returns `rusqlite::Result` instead of panicking. A migration
/// failure (e.g. an I/O error mid-`ALTER TABLE` on a full disk) now propagates
/// to [`UniversePool::try_get_or_open`], which marks that single universe
/// unavailable rather than aborting the whole process at boot.
fn run_universe_migrations(conn: &Connection, universe_key: &str) -> rusqlite::Result<()> {
    conn.execute_batch(UNIVERSE_SCHEMA)?;

    let v: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if v < 1 {
        conn.execute("INSERT INTO schema_version (version) VALUES (1)", [])?;
    }
    if v < 2 {
        // CO-73: entry_dates table already created via UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (2)", [])?;
    }
    if v < 3 {
        // CO-74: entry_relations table already created via UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (3)", [])?;
    }
    if v < 4 {
        // CO-146: assets table already created via UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (4)", [])?;
    }
    if v < 5 {
        // CO-147: asset_tags + frontmatter_index already created via
        // UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (5)", [])?;
    }
    if v < 6 {
        // CO-148: encryption envelope columns on assets.
        // `encrypted = 0` rows are Phase 1 plaintext blobs (stay readable).
        // `encrypted = 1` rows are ChaCha20-Poly1305 ciphertext with `nonce`
        // populated and `cipher_size` reflecting the on-disk byte count.
        ensure_universe_column(conn, "assets", "nonce", "BLOB")?;
        ensure_universe_column(conn, "assets", "cipher_size", "INTEGER")?;
        ensure_universe_column(conn, "assets", "encrypted", "INTEGER NOT NULL DEFAULT 0")?;
        conn.execute("INSERT INTO schema_version (version) VALUES (6)", [])?;
    }
    if v < 7 {
        // CO-156: references_meta + references_fts already created via UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (7)", [])?;
    }
    if v < 8 {
        // CO-158: add work_id, edition_id, primary_layer; change PK to
        // (universe_key, entry_path, edition_id).
        // If edition_id column is absent, the table has the old v7 schema and
        // must be recreated. New databases already have the v8 schema from
        // UNIVERSE_SCHEMA, so the check is a no-op for them.
        if !universe_column_exists(conn, "references_meta", "edition_id") {
            recreate_references_meta_v8(conn)?;
        }
        conn.execute("INSERT INTO schema_version (version) VALUES (8)", [])?;
    }
    if v < 9 {
        // Temporal validity on entry_relations: edges valid only within
        // [valid_from, valid_until). NULL on either side means unbounded.
        // Sourced from the *from-entry's* frontmatter `valid_from`/`valid_until`
        // and applied to all of its outbound edges (entry-level lifecycle).
        ensure_universe_column(conn, "entry_relations", "valid_from", "TEXT")?;
        ensure_universe_column(conn, "entry_relations", "valid_until", "TEXT")?;
        conn.execute("INSERT INTO schema_version (version) VALUES (9)", [])?;
    }
    if v < 10 {
        // CO-164: vector embedding store — already created via UNIVERSE_SCHEMA IF NOT EXISTS.
        conn.execute("INSERT INTO schema_version (version) VALUES (10)", [])?;
    }
    if v < 11 {
        // 2.7.25: append-only event log per universe. Every PUT/DELETE
        // becomes an immutable row here, written in the same SQLite
        // transaction as the `entries` upsert. Source of truth for
        // time-travel queries; future Kafka/Iceberg exporters read
        // from this table (or pump it through a worker).
        //
        // Schema designed to map 1:1 to a `co.v1.EntryEvent` protobuf:
        //   seq            — strictly monotonic per universe
        //   ts_micros      — Unix epoch in microseconds (orderable across replicas)
        //   op             — 'put' | 'delete'
        //   path           — entry path
        //   body_hash      — BLAKE3 of new body; NULL on delete
        //   prev_body_hash — BLAKE3 of old body (NULL on first write or delete-of-absent)
        //   body           — snapshot of body bytes (for replay without blob lookup)
        //   frontmatter_json — snapshot of frontmatter at write time
        //   author_id      — user_id when authed, NULL for anon writes
        //   request_id     — idempotency key supplied by caller; UNIQUE when non-null
        //   exported_at    — set by Kafka/Iceberg worker when it has shipped this event
        //
        // request_id is INDEXED unique only when non-null (SQLite partial index).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entry_events (
                seq               INTEGER PRIMARY KEY AUTOINCREMENT,
                ts_micros         INTEGER NOT NULL,
                op                TEXT NOT NULL CHECK(op IN ('put','delete')),
                path              TEXT NOT NULL,
                body_hash         TEXT,
                prev_body_hash    TEXT,
                body              BLOB,
                frontmatter_json  TEXT,
                author_id         TEXT,
                request_id        TEXT,
                exported_at       TEXT
            );
             CREATE INDEX IF NOT EXISTS idx_entry_events_path_ts
                ON entry_events(path, ts_micros DESC);
             CREATE INDEX IF NOT EXISTS idx_entry_events_ts
                ON entry_events(ts_micros);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_entry_events_request_id
                ON entry_events(request_id)
                WHERE request_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_entry_events_unexported
                ON entry_events(seq)
                WHERE exported_at IS NULL;",
        )?;
        conn.execute("INSERT INTO schema_version (version) VALUES (11)", [])?;
    }
    if v < 12 {
        // CO-241: true content-volume metrics — lines/words/chars separate
        // from content_count (file count). New databases already have these
        // columns from UNIVERSE_SCHEMA; existing DBs get them via ALTER TABLE.
        ensure_universe_column(conn, "entries", "body_lines", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_universe_column(conn, "entries", "body_words", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_universe_column(conn, "entries", "body_chars", "INTEGER NOT NULL DEFAULT 0")?;
        // Backfill: compute the three metrics for every row that still has the
        // DEFAULT 0 values. SQLite's length() gives char count; lines and words
        // require a Rust-side pass (no built-in). We mark them "needs backfill"
        // by leaving body_chars = 0, then the unconditional backfill below fixes
        // them up. Inserting schema_version 12 here lets us skip the ALTER TABLE
        // on future opens.
        conn.execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (12)",
            [],
        )?;
    }
    if v < 13 {
        // CO-242: backfill entries rows for existing assets so they appear in the
        // unified listing. One row per asset using path = 'attachments/<sha256>'.
        // entry_type is derived from the MIME column. INSERT OR IGNORE so re-running
        // the migration on an already-migrated DB is a no-op.
        let type_case = "CASE \
            WHEN mime = 'application/pdf' THEN 'asset.pdf' \
            WHEN mime LIKE 'image/%' THEN 'asset.image' \
            WHEN mime LIKE 'video/%' THEN 'asset.video' \
            WHEN mime LIKE 'text/%' \
              OR mime IN ('application/json','application/javascript','application/typescript',\
                          'application/xml','application/yaml','application/x-yaml') \
              THEN 'asset.code' \
            ELSE 'asset.binary' \
        END";
        conn.execute(
            &format!(
                "INSERT OR IGNORE INTO entries \
                   (path, universe_key, entry_type, title, \
                    frontmatter_json, payload, body, body_hash, \
                    body_lines, body_words, body_chars, \
                    created_at, updated_at) \
                 SELECT \
                   'attachments/' || sha256, \
                   ?1, \
                   {type_case}, \
                   COALESCE(filename, substr(sha256, 1, 16)), \
                   json_object( \
                     'type', {type_case}, \
                     'mime', mime, \
                     'asset_sha256', sha256, \
                     'size_bytes', size_bytes, \
                     'filename', COALESCE(filename, '') \
                   ), \
                   json_object( \
                     'type', {type_case}, \
                     'mime', mime, \
                     'asset_sha256', sha256, \
                     'size_bytes', size_bytes, \
                     'filename', COALESCE(filename, '') \
                   ), \
                   '', '', 0, 0, 0, \
                   datetime(created_at_ns / 1000000000, 'unixepoch'), \
                   datetime(created_at_ns / 1000000000, 'unixepoch') \
                 FROM assets"
            ),
            rusqlite::params![universe_key],
        )?;
        conn.execute("INSERT INTO schema_version (version) VALUES (13)", [])?;
    }
    if v < 14 {
        // CO-267: entry_origin — distinguishes seed-walker writes ('walker')
        // from co-sync push writes ('synced'). Default '' for existing rows
        // (treated as walker-equivalent by the upsert guard).
        ensure_universe_column(conn, "entries", "entry_origin", "TEXT NOT NULL DEFAULT ''")?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (14)",
            [],
        )?;
    }
    // CO-267 unconditional backfill — same drift-safe guard as CO-241.
    ensure_universe_column(conn, "entries", "entry_origin", "TEXT NOT NULL DEFAULT ''")?;
    if v < 15 {
        // CO-330: functional index on frontmatter_json.published for the anon published-only filter.
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_entries_published \
             ON entries(universe_key, json_extract(frontmatter_json, '$.published')); \
             INSERT OR IGNORE INTO schema_version (version) VALUES (15);",
        )?;
    }
    if v < 16 {
        // CO-363: link_text column on entry_relations — stores optional alias label
        // from [[target|label]] wikilinks. Nullable; NULL means no alias was written.
        // New universes already have the column from UNIVERSE_SCHEMA; existing ones
        // get it here.
        ensure_universe_column(conn, "entry_relations", "link_text", "TEXT")?;
        // Backfill body wikilinks for all existing entries. This is a one-time
        // ~60s run for ~11,500-entry universes. Uses INSERT OR REPLACE scoped to
        // wikilink relation_types so frontmatter FK rows are not touched.
        if let Err(e) = crate::content::relation_index::backfill_body_wikilinks(conn, universe_key)
        {
            tracing::warn!(
                universe = %universe_key,
                error = %e,
                "CO-363: body wikilink backfill failed (non-fatal)"
            );
        }
        conn.execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (16)",
            [],
        )?;
    }
    // CO-363 unconditional drift guard — ensures link_text exists even if v16
    // was partially applied on an older instance.
    ensure_universe_column(conn, "entry_relations", "link_text", "TEXT")?;

    if v < 17 {
        // CO-389: source_marker — diagnostic column tracking which channel last wrote
        // this entry: 'yggdrasil-live' (live overlay), 'remote-git' (CO-337 poll),
        // 'vault-write' (Vault API write), or NULL (legacy / untracked).
        ensure_universe_column(conn, "entries", "source_marker", "TEXT")?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (17)",
            [],
        )?;
    }
    // CO-389 unconditional drift guard.
    ensure_universe_column(conn, "entries", "source_marker", "TEXT")?;

    if v < 18 {
        // CO-354: suggest/review lifecycle. Every entry gains a review_status
        // (draft | reviewed | published) plus submitter/reviewer provenance.
        // Default 'published' so existing rows are unaffected — purely additive.
        // New universes already have the columns from UNIVERSE_SCHEMA; existing
        // ones get them here via ALTER TABLE. (v18 — v17 is CO-389 on main.)
        ensure_universe_column(
            conn,
            "entries",
            "review_status",
            "TEXT NOT NULL DEFAULT 'published'",
        )?;
        ensure_universe_column(conn, "entries", "submitted_by", "TEXT")?;
        ensure_universe_column(conn, "entries", "submitted_at", "TEXT")?;
        ensure_universe_column(conn, "entries", "reviewed_by", "TEXT")?;
        ensure_universe_column(conn, "entries", "reviewed_at", "TEXT")?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_entries_review_status \
             ON entries(universe_key, review_status) WHERE review_status != 'published'; \
             INSERT OR IGNORE INTO schema_version (version) VALUES (18);",
        )?;
    }
    if v < 19 {
        // CO-387: canonical Unix-ms columns for the time-rendering primitive.
        // ISO strings in `entry_dates` (CO-73) remain the durable source of
        // truth; these columns are the per-lens math fast path (i64 ms covers
        // ±292 Myr — human / historical / fictional / Pomodoro scales; cosmic
        // scale uses frontmatter_json fields instead, see CO-387 spec).
        // New universes already have the columns from UNIVERSE_SCHEMA;
        // existing ones get them here via ALTER TABLE.
        ensure_universe_column(conn, "entries", "event_at_ms", "INTEGER")?;
        ensure_universe_column(conn, "entries", "due_at_ms", "INTEGER")?;
        ensure_universe_column(conn, "entries", "scheduled_at_ms", "INTEGER")?;
        // Backfill from entry_dates ISO values. julianday() keeps millisecond
        // precision (strftime('%s', …) floors to whole seconds) and handles
        // pre-epoch dates (negative ms).
        for (semantic, column) in [
            ("event_at", "event_at_ms"),
            ("due_at", "due_at_ms"),
            ("scheduled_at", "scheduled_at_ms"),
        ] {
            conn.execute(
                &format!(
                    "UPDATE entries SET {column} = (
                        SELECT CAST(ROUND((julianday(ed.value) - 2440587.5) * 86400000.0) AS INTEGER)
                        FROM entry_dates ed
                        WHERE ed.universe_key = entries.universe_key
                          AND ed.entry_path   = entries.path
                          AND ed.semantic     = '{semantic}'
                    )
                    WHERE {column} IS NULL
                      AND EXISTS (
                        SELECT 1 FROM entry_dates ed
                        WHERE ed.universe_key = entries.universe_key
                          AND ed.entry_path   = entries.path
                          AND ed.semantic     = '{semantic}'
                      )"
                ),
                [],
            )?;
        }
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_entries_event_ms \
             ON entries(universe_key, event_at_ms) WHERE event_at_ms IS NOT NULL; \
             INSERT OR IGNORE INTO schema_version (version) VALUES (19);",
        )?;
    }
    // CO-387 unconditional drift guard — same partial-apply safety as CO-241/CO-267.
    ensure_universe_column(conn, "entries", "event_at_ms", "INTEGER")?;
    ensure_universe_column(conn, "entries", "due_at_ms", "INTEGER")?;
    ensure_universe_column(conn, "entries", "scheduled_at_ms", "INTEGER")?;

    // CO-354 unconditional drift guard — same partial-apply safety as CO-241/CO-267.
    ensure_universe_column(
        conn,
        "entries",
        "review_status",
        "TEXT NOT NULL DEFAULT 'published'",
    )?;
    ensure_universe_column(conn, "entries", "submitted_by", "TEXT")?;
    ensure_universe_column(conn, "entries", "submitted_at", "TEXT")?;
    ensure_universe_column(conn, "entries", "reviewed_by", "TEXT")?;
    ensure_universe_column(conn, "entries", "reviewed_at", "TEXT")?;

    // CO-241 unconditional backfill: for every entry where body_chars = 0 and
    // body IS NOT '' we cannot distinguish "genuinely empty body" from "not yet
    // computed", so we re-run the computation for all rows where body_chars = 0.
    // Rows with an empty body legitimately have body_chars = 0 and will be
    // re-set to 0, which is a no-op. Idempotent.
    {
        let mut stmt =
            conn.prepare("SELECT rowid, body FROM entries WHERE body_chars = 0 AND body != ''")?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        for (rowid, body) in rows {
            let lines = body.lines().count() as i64;
            let words = body.split_whitespace().count() as i64;
            let chars = body.chars().count() as i64;
            conn.execute(
                "UPDATE entries SET body_lines = ?1, body_words = ?2, body_chars = ?3 \
                 WHERE rowid = ?4",
                rusqlite::params![lines, words, chars, rowid],
            )?;
        }
    }

    Ok(())
}

/// Exposed for integration tests that need a fully-migrated per-universe DB.
#[doc(hidden)]
pub fn run_universe_migrations_for_test(conn: &Connection) {
    run_universe_migrations(conn, "test-universe").expect("test migrations");
}

fn universe_column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    conn.query_row(
        &format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"),
        rusqlite::params![column],
        |_| Ok(true),
    )
    .ok()
    .unwrap_or(false)
}

/// Recreate `references_meta` with the CO-158 schema (3-part PK, work_id, edition_id,
/// primary_layer), backfilling `work_id` from the entry_path stem and `edition_id =
/// 'default'` for all existing rows.
fn recreate_references_meta_v8(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS references_meta_v8 (
            universe_key  TEXT NOT NULL,
            entry_path    TEXT NOT NULL,
            edition_id    TEXT NOT NULL DEFAULT 'default',
            work_id       TEXT NOT NULL DEFAULT '',
            primary_layer INTEGER,
            file          TEXT,
            blob_sha256   TEXT,
            url           TEXT,
            medium        TEXT NOT NULL DEFAULT 'unknown',
            mime          TEXT,
            size_bytes    INTEGER,
            language      TEXT,
            seed_status   TEXT NOT NULL DEFAULT 'stub',
            indexed_at    TEXT NOT NULL,
            PRIMARY KEY (universe_key, entry_path, edition_id)
        );",
    )?;

    struct RefRow {
        universe_key: String,
        entry_path: String,
        file: Option<String>,
        blob_sha256: Option<String>,
        url: Option<String>,
        medium: String,
        mime: Option<String>,
        size_bytes: Option<i64>,
        language: Option<String>,
        seed_status: String,
        indexed_at: String,
    }

    let rows: Vec<RefRow> = {
        let mut stmt = conn.prepare(
            "SELECT universe_key, entry_path, file, blob_sha256, url, \
                 medium, mime, size_bytes, language, seed_status, indexed_at \
                 FROM references_meta",
        )?;
        stmt.query_map([], |row| {
            Ok(RefRow {
                universe_key: row.get(0)?,
                entry_path: row.get(1)?,
                file: row.get(2)?,
                blob_sha256: row.get(3)?,
                url: row.get(4)?,
                medium: row.get(5)?,
                mime: row.get(6)?,
                size_bytes: row.get(7)?,
                language: row.get(8)?,
                seed_status: row.get(9)?,
                indexed_at: row.get(10)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect()
    };

    for row in rows {
        let work_id = ref_work_id_from_path(&row.entry_path);
        conn.execute(
            "INSERT OR IGNORE INTO references_meta_v8 \
             (universe_key, entry_path, edition_id, work_id, primary_layer, \
              file, blob_sha256, url, medium, mime, size_bytes, language, seed_status, indexed_at) \
             VALUES (?1, ?2, 'default', ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                row.universe_key,
                row.entry_path,
                work_id,
                row.file,
                row.blob_sha256,
                row.url,
                row.medium,
                row.mime,
                row.size_bytes,
                row.language,
                row.seed_status,
                row.indexed_at,
            ],
        )?;
    }

    conn.execute_batch(
        "DROP TABLE references_meta;
         ALTER TABLE references_meta_v8 RENAME TO references_meta;
         CREATE INDEX IF NOT EXISTS idx_refs_medium      ON references_meta(medium);
         CREATE INDEX IF NOT EXISTS idx_refs_seed_status ON references_meta(seed_status);
         CREATE INDEX IF NOT EXISTS idx_refs_blob
             ON references_meta(blob_sha256) WHERE blob_sha256 IS NOT NULL;
         CREATE INDEX IF NOT EXISTS idx_refs_work_id     ON references_meta(work_id);
         CREATE INDEX IF NOT EXISTS idx_refs_primary_layer
             ON references_meta(primary_layer) WHERE primary_layer IS NOT NULL;",
    )?;
    Ok(())
}

/// Derive a work_id slug from an entry path: take the filename stem.
/// e.g. "refs/GNDicLex.md" → "GNDicLex"
fn ref_work_id_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Per-universe DB version of the storage.rs `ensure_column` helper. Mirrors
/// the CO-137 drift-safe pattern.
fn ensure_universe_column(
    conn: &Connection,
    table: &str,
    column: &str,
    def: &str,
) -> rusqlite::Result<()> {
    let exists: bool = conn
        .query_row(
            &format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"),
            rusqlite::params![column],
            |_| Ok(true),
        )
        .ok()
        .unwrap_or(false);
    if !exists {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {def};"))?;
    }
    Ok(())
}

#[cfg(test)]
mod time_migration_tests {
    use super::*;

    /// CO-387 v19: canonical ms columns are added to `entries` and backfilled
    /// from the CO-73 `entry_dates` ISO rows (semantic = event_at / due_at /
    /// scheduled_at). ISO strings stay the durable source (entry_dates); the
    /// ms columns are the lens-math fast path.
    #[test]
    fn v19_adds_ms_columns_and_backfills_from_entry_dates() {
        let conn = Connection::open_in_memory().unwrap();
        // Pre-v19 database: entries without ms columns + entry_dates rows.
        conn.execute_batch(
            "CREATE TABLE entries (
                universe_key      TEXT NOT NULL,
                path              TEXT NOT NULL,
                entry_type        TEXT NOT NULL DEFAULT 'note',
                title             TEXT NOT NULL DEFAULT '',
                body              TEXT NOT NULL DEFAULT '',
                frontmatter_json  TEXT NOT NULL DEFAULT '{}',
                body_chars        INTEGER NOT NULL DEFAULT 0,
                created_at        TEXT NOT NULL DEFAULT '',
                updated_at        TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (universe_key, path)
            );
            CREATE TABLE entry_dates (
                universe_key TEXT NOT NULL,
                entry_path   TEXT NOT NULL,
                semantic     TEXT NOT NULL,
                value        TEXT NOT NULL,
                PRIMARY KEY (universe_key, entry_path, semantic)
            );
            CREATE TABLE schema_version (version INTEGER NOT NULL);
            INSERT INTO schema_version (version) VALUES (18);
            INSERT INTO entries (universe_key, path) VALUES ('uni', 'events/moon.md');
            INSERT INTO entry_dates VALUES ('uni', 'events/moon.md', 'event_at', '1969-07-20T20:17:00Z');
            INSERT INTO entry_dates VALUES ('uni', 'events/moon.md', 'due_at',   '1970-01-01T00:00:01Z');",
        )
        .unwrap();

        run_universe_migrations(&conn, "uni").unwrap();

        assert!(universe_column_exists(&conn, "entries", "event_at_ms"));
        assert!(universe_column_exists(&conn, "entries", "due_at_ms"));
        assert!(universe_column_exists(&conn, "entries", "scheduled_at_ms"));

        let (event_ms, due_ms, sched_ms): (Option<i64>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT event_at_ms, due_at_ms, scheduled_at_ms FROM entries \
                 WHERE universe_key='uni' AND path='events/moon.md'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        // 1969-07-20T20:17:00Z = -14_182_980_000 ms (pre-epoch dates must work)
        assert_eq!(event_ms, Some(-14_182_980_000));
        assert_eq!(due_ms, Some(1_000));
        assert_eq!(sched_ms, None);

        // v19 must be recorded and the supporting index created.
        let v: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert!(v >= 19, "schema_version must reach v19, got {v}");
        let idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_entries_event_ms'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1, "v19 must create idx_entries_event_ms");
    }

    /// Fresh databases get the ms columns from the base schema and the
    /// backfill is a no-op (idempotent re-run safety).
    #[test]
    fn v19_is_idempotent_on_fresh_database() {
        let conn = Connection::open_in_memory().unwrap();
        run_universe_migrations(&conn, "uni").unwrap();
        run_universe_migrations(&conn, "uni").unwrap(); // re-run must not panic
        assert!(universe_column_exists(&conn, "entries", "event_at_ms"));
    }
}

#[cfg(test)]
mod base_schema_tests {
    use super::*;

    /// Regression (2026-06-10 staging boot crash): the base UNIVERSE_SCHEMA
    /// batch runs on EXISTING databases whose `entries` table predates newer
    /// columns. Any statement in the batch that references a
    /// migration-added column (e.g. CO-354's review_status partial index)
    /// fails the whole batch and panics the server before migrations run.
    /// Simulate a pre-CO-354 database and assert open+migrate succeeds.
    #[test]
    fn base_schema_batch_survives_pre_co354_entries_table() {
        let conn = Connection::open_in_memory().unwrap();
        // Old entries table: no review_status / submitted_* / reviewed_* /
        // source_marker columns — the shape deployed before v17/v18.
        conn.execute_batch(
            "CREATE TABLE entries (
                universe_key      TEXT NOT NULL,
                path              TEXT NOT NULL,
                entry_type        TEXT NOT NULL DEFAULT 'note',
                title             TEXT NOT NULL DEFAULT '',
                body              TEXT NOT NULL DEFAULT '',
                frontmatter_json  TEXT NOT NULL DEFAULT '{}',
                body_chars        INTEGER NOT NULL DEFAULT 0,
                created_at        TEXT NOT NULL DEFAULT '',
                updated_at        TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (universe_key, path)
            );
            CREATE TABLE schema_version (version INTEGER NOT NULL);
            INSERT INTO schema_version (version) VALUES (16);",
        )
        .unwrap();

        // Must not panic: base batch is column-agnostic; v17/v18 add columns
        // and only then create the dependent index.
        run_universe_migrations(&conn, "legacy-universe").unwrap();

        assert!(universe_column_exists(&conn, "entries", "review_status"));
        let idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_entries_review_status'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1, "v18 must create the review_status partial index");
    }
}

#[cfg(test)]
mod degradation_tests {
    use super::*;

    /// Plant a regular FILE where universe `key`'s directory should be, so that
    /// `create_dir_all(parent)` for `data.db` fails — a deterministic stand-in
    /// for the disk-full / I/O-error / corrupt-`-shm` conditions that took prod
    /// down on 2026-06-11.
    fn poison_universe(pool: &UniversePool, key: &str) {
        let dir = pool.universe_dir(key);
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        std::fs::write(&dir, b"not a directory").unwrap();
    }

    /// CO-406: a single universe whose pool open fails must NOT panic. It is
    /// marked unavailable and returns `Err`, while every OTHER universe opens
    /// and serves normally — no crash-loop.
    #[test]
    fn one_bad_universe_does_not_take_down_the_pool() {
        let dir = tempfile::tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);

        poison_universe(&pool, "broken");

        // The bad universe fails gracefully (no panic) and is recorded.
        let bad = pool.try_get_or_open("broken");
        assert!(bad.is_err(), "poisoned universe must return Err, not panic");
        assert!(
            pool.unavailable_reason("broken").is_some(),
            "failure must be recorded so routes can 503 + atividades can log it"
        );

        // A healthy universe opens fine despite the broken neighbour.
        let good = pool.try_get_or_open("healthy");
        assert!(good.is_ok(), "other universes must keep serving");
        assert!(pool.unavailable_reason("healthy").is_none());
    }

    /// CO-406: once the environment failure clears, the next access (or an
    /// admin `reopen`) recovers the universe WITHOUT a process restart.
    #[test]
    fn unavailable_universe_recovers_on_next_access() {
        let dir = tempfile::tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);

        poison_universe(&pool, "u");
        assert!(pool.try_get_or_open("u").is_err());
        assert!(pool.unavailable_reason("u").is_some());

        // Clear the environment failure (remove the blocking file).
        std::fs::remove_file(pool.universe_dir("u")).unwrap();

        // Lazy retry: the next access succeeds and clears the unavailable mark.
        let recovered = pool.try_get_or_open("u");
        assert!(recovered.is_ok(), "universe must recover on next access");
        assert!(
            pool.unavailable_reason("u").is_none(),
            "successful reopen must clear the unavailable mark"
        );
    }

    /// CO-406: the panicking `get_or_open` convenience wrapper still works for
    /// the healthy internal call sites (seeding/recompute/clone) — it delegates
    /// to the same non-panicking core.
    #[test]
    fn get_or_open_wrapper_opens_healthy_universe() {
        let dir = tempfile::tempdir().unwrap();
        let pool = UniversePool::new(dir.path(), 16);
        let _conn = pool.get_or_open("ok");
        assert_eq!(pool.open_count(), 1);
    }

    /// CO-433 (per-universe sharding): a write held open on universe A must NOT
    /// block a concurrent write to universe B. Each universe owns its own
    /// `Arc<Mutex<Connection>>`, so they shard rather than serialize behind one
    /// global lock.
    ///
    /// Proof: the main thread holds universe A's connection lock for the whole
    /// test, then a spawned thread performs a full write against universe B and
    /// signals completion. If B's write were serialized behind A's lock, the
    /// `recv_timeout` would elapse (deadlock); it returns promptly instead.
    #[test]
    fn writes_to_two_universes_do_not_serialize() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(UniversePool::new(dir.path(), 16));

        // Hold universe A's connection lock for the entire test.
        let conn_a = pool.try_get_or_open("uni-a").unwrap();
        let guard_a = conn_a.lock().expect("lock A");

        // Spawn a thread that writes to universe B via its own EntryStore.
        let (tx, rx) = mpsc::channel();
        let pool_b = Arc::clone(&pool);
        let handle = std::thread::spawn(move || {
            let store_b = pool_b.entry_store("uni-b").expect("open B store");
            let entry = crate::domain::EntryDomain {
                path: "n.md".into(),
                universe_key: "uni-b".into(),
                entry_type: "note".into(),
                title: Some("n".into()),
                frontmatter: serde_json::json!({"title": "n"}),
                body: "b".into(),
                body_hash: String::new(),
                created_at: None,
                updated_at: None,
            };
            store_b.upsert(&entry).expect("write B");
            tx.send(()).expect("signal");
        });

        // B's write must complete while A's lock is still held.
        rx.recv_timeout(Duration::from_secs(5))
            .expect("universe B write blocked on universe A's lock — universes are serializing");

        handle.join().unwrap();
        drop(guard_a);

        // B's write actually landed in B (and never touched A).
        let store_b = pool.entry_store("uni-b").unwrap();
        assert!(store_b.get("n.md").unwrap().is_some());
        let store_a = pool.entry_store("uni-a").unwrap();
        assert!(store_a.get("n.md").unwrap().is_none());
    }
}
