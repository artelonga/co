//! Reference repository — data-access trait + SQLite implementation.
//!
//! CO-432: propagates the CO-390 repository pattern from `entries` to
//! `references`. Hides both reference projections from handlers:
//! `references_meta` (cards, CO-156/158) and `references_index` (frontmatter
//! references + excerpts, CO-154).
//!
//! The SQLite implementation wraps the per-universe connection
//! (`Arc<std::sync::Mutex<Connection>>` — the type returned by
//! `Storage::universe_conn`) and keeps multi-statement card writes inside a
//! single lock scope, exactly as the pre-432 handlers did.

use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, params};

use crate::content::references::meta::{remove_reference_meta, upsert_reference_meta};
use crate::domain::ReferenceDomain;
use crate::entry_index::EntryIndex;
use crate::reference_index::{ReferenceIndex, ReferenceRow};

// ---------------------------------------------------------------------------
// Filter + record types
// ---------------------------------------------------------------------------

/// Structured filter for listing reference cards.
#[derive(Debug, Default, Clone)]
pub struct CardFilter {
    pub medium: Option<String>,
    pub seed_status: Option<String>,
    pub work_id: Option<String>,
    pub primary_layer: Option<i64>,
    /// Full-text query across title, body, transcription. When present, the
    /// structured filters are ignored (FTS path), mirroring the API contract.
    pub fts: Option<String>,
}

/// An asset row with no corresponding reference card.
#[derive(Debug, Clone)]
pub struct OrphanBlobRecord {
    pub sha256: String,
    pub mime: String,
    pub size_bytes: i64,
    pub filename: Option<String>,
}

// ---------------------------------------------------------------------------
// Repository trait
// ---------------------------------------------------------------------------

/// Data-access abstraction for reference cards and the references index.
pub trait ReferenceRepository: Send + Sync {
    /// List card editions matching `filter`. FTS results cap at 200 rows,
    /// structured results at 500 (pre-432 API behavior).
    fn list_cards(&self, universe_key: &str, filter: &CardFilter) -> Result<Vec<ReferenceDomain>>;

    /// The canonical edition of one card (edition `default` first, then by
    /// edition_id). `None` when the card has no row.
    fn get_card(&self, universe_key: &str, path: &str) -> Result<Option<ReferenceDomain>>;

    /// Assets with no corresponding reference card (cap 500).
    fn orphan_blobs(&self, universe_key: &str) -> Result<Vec<OrphanBlobRecord>>;

    /// `(entry_path, file)` for every file-bound card row.
    fn card_files(&self, universe_key: &str) -> Result<Vec<(String, String)>>;

    /// Distinct non-empty `work_id` values, sorted.
    fn list_works(&self, universe_key: &str) -> Result<Vec<String>>;

    /// Upsert a card: entry row + `references_meta`/FTS projections, all
    /// within one connection lock scope.
    fn upsert_card(
        &self,
        universe_key: &str,
        entry: &co::entry::Entry,
        frontmatter: &serde_json::Value,
        body: &str,
        title: Option<&str>,
        universe_root: &std::path::Path,
    ) -> Result<()>;

    /// Delete a card: entry row + meta/FTS projections, one lock scope.
    fn delete_card(&self, universe_key: &str, path: &str) -> Result<()>;

    /// CO-154: query the frontmatter references index.
    fn query_refs(
        &self,
        universe_key: &str,
        source_contains: Option<&str>,
        url_contains: Option<&str>,
        fts_query: Option<&str>,
    ) -> Result<Vec<ReferenceRow>>;

    /// CO-154: wikilinks inside `## Referência:` excerpts with no matching entry.
    fn orphan_wikilinks(&self, universe_key: &str) -> Result<Vec<String>>;
}

// ---------------------------------------------------------------------------
// SQLite implementation
// ---------------------------------------------------------------------------

const CARD_SELECT: &str = "SELECT rm.universe_key, rm.entry_path, rm.edition_id, rm.work_id, rm.primary_layer, \
            rm.file, rm.blob_sha256, rm.url, rm.medium, rm.mime, rm.size_bytes, \
            rm.language, rm.seed_status, rm.indexed_at, e.title \
     FROM references_meta rm \
     LEFT JOIN entries e \
       ON e.universe_key = rm.universe_key AND e.path = rm.entry_path";

fn row_to_domain(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReferenceDomain> {
    Ok(ReferenceDomain {
        universe_key: row.get(0)?,
        entry_path: row.get(1)?,
        edition_id: row.get(2)?,
        work_id: row.get(3)?,
        primary_layer: row.get(4)?,
        file: row.get(5)?,
        blob_sha256: row.get(6)?,
        url: row.get(7)?,
        medium: row.get(8)?,
        mime: row.get(9)?,
        size_bytes: row.get(10)?,
        language: row.get(11)?,
        seed_status: row.get(12)?,
        indexed_at: row.get(13)?,
        title: row.get(14)?,
    })
}

/// Production repository — wraps the reference projections behind the trait.
pub struct SqliteReferenceRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteReferenceRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow!("universe conn lock poisoned"))
    }
}

impl ReferenceRepository for SqliteReferenceRepository {
    fn list_cards(&self, universe_key: &str, filter: &CardFilter) -> Result<Vec<ReferenceDomain>> {
        let guard = self.lock()?;

        if let Some(ref fts_query) = filter.fts {
            let fts_sql = format!(
                "{CARD_SELECT} \
                 JOIN reference_cards_fts fts \
                   ON fts.universe_key = rm.universe_key AND fts.entry_path = rm.entry_path \
                 WHERE fts.universe_key = ?1 AND reference_cards_fts MATCH ?2 \
                 ORDER BY rm.entry_path, rm.edition_id LIMIT 200"
            );
            let mut stmt = guard.prepare(&fts_sql)?;
            let cards = stmt
                .query_map(params![universe_key, fts_query], row_to_domain)?
                .filter_map(|r| r.ok())
                .collect();
            return Ok(cards);
        }

        let mut sql = format!("{CARD_SELECT} WHERE rm.universe_key = ?1");
        let mut bind_vals: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(universe_key.to_string())];

        if let Some(ref m) = filter.medium {
            bind_vals.push(Box::new(m.clone()));
            sql.push_str(&format!(" AND rm.medium = ?{}", bind_vals.len()));
        }
        if let Some(ref s) = filter.seed_status {
            bind_vals.push(Box::new(s.clone()));
            sql.push_str(&format!(" AND rm.seed_status = ?{}", bind_vals.len()));
        }
        if let Some(ref w) = filter.work_id {
            bind_vals.push(Box::new(w.clone()));
            sql.push_str(&format!(" AND rm.work_id = ?{}", bind_vals.len()));
        }
        if let Some(pl) = filter.primary_layer {
            bind_vals.push(Box::new(pl));
            sql.push_str(&format!(" AND rm.primary_layer = ?{}", bind_vals.len()));
        }
        sql.push_str(" ORDER BY rm.entry_path, rm.edition_id LIMIT 500");

        let params_refs: Vec<&dyn rusqlite::types::ToSql> = bind_vals
            .iter()
            .map(|b| b.as_ref() as &dyn rusqlite::types::ToSql)
            .collect();
        let mut stmt = guard.prepare(&sql)?;
        let cards = stmt
            .query_map(params_refs.as_slice(), row_to_domain)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(cards)
    }

    fn get_card(&self, universe_key: &str, path: &str) -> Result<Option<ReferenceDomain>> {
        let guard = self.lock()?;
        let sql = format!(
            "{CARD_SELECT} \
             WHERE rm.universe_key = ?1 AND rm.entry_path = ?2 \
             ORDER BY CASE rm.edition_id WHEN 'default' THEN 0 ELSE 1 END, rm.edition_id \
             LIMIT 1"
        );
        match guard.query_row(&sql, params![universe_key, path], row_to_domain) {
            Ok(card) => Ok(Some(card)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn orphan_blobs(&self, universe_key: &str) -> Result<Vec<OrphanBlobRecord>> {
        let guard = self.lock()?;
        let mut stmt = guard.prepare(
            "SELECT a.sha256, a.mime, a.size_bytes, a.filename \
             FROM assets a \
             WHERE NOT EXISTS ( \
               SELECT 1 FROM references_meta rm \
               WHERE rm.universe_key = ?1 AND rm.blob_sha256 = a.sha256 \
             ) \
             ORDER BY a.sha256 LIMIT 500",
        )?;
        let blobs = stmt
            .query_map(params![universe_key], |row| {
                Ok(OrphanBlobRecord {
                    sha256: row.get(0)?,
                    mime: row.get(1)?,
                    size_bytes: row.get(2)?,
                    filename: row.get(3)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(blobs)
    }

    fn card_files(&self, universe_key: &str) -> Result<Vec<(String, String)>> {
        let guard = self.lock()?;
        let mut stmt = guard.prepare(
            "SELECT entry_path, file FROM references_meta \
             WHERE universe_key = ?1 AND file IS NOT NULL",
        )?;
        let rows = stmt
            .query_map(params![universe_key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    fn list_works(&self, universe_key: &str) -> Result<Vec<String>> {
        let guard = self.lock()?;
        let mut stmt = guard.prepare(
            "SELECT DISTINCT work_id FROM references_meta \
             WHERE universe_key = ?1 AND work_id != '' \
             ORDER BY work_id",
        )?;
        let works = stmt
            .query_map(params![universe_key], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(works)
    }

    fn upsert_card(
        &self,
        universe_key: &str,
        entry: &co::entry::Entry,
        frontmatter: &serde_json::Value,
        body: &str,
        title: Option<&str>,
        universe_root: &std::path::Path,
    ) -> Result<()> {
        let guard = self.lock()?;
        EntryIndex::new(&guard).upsert(universe_key, entry)?;
        upsert_reference_meta(
            &guard,
            universe_key,
            &entry.path,
            frontmatter,
            body,
            title,
            universe_root,
        );
        Ok(())
    }

    fn delete_card(&self, universe_key: &str, path: &str) -> Result<()> {
        let guard = self.lock()?;
        EntryIndex::new(&guard).remove(universe_key, path)?;
        remove_reference_meta(&guard, universe_key, path);
        Ok(())
    }

    fn query_refs(
        &self,
        universe_key: &str,
        source_contains: Option<&str>,
        url_contains: Option<&str>,
        fts_query: Option<&str>,
    ) -> Result<Vec<ReferenceRow>> {
        let guard = self.lock()?;
        ReferenceIndex::new(&guard).query(universe_key, source_contains, url_contains, fts_query)
    }

    fn orphan_wikilinks(&self, universe_key: &str) -> Result<Vec<String>> {
        let guard = self.lock()?;
        ReferenceIndex::new(&guard).orphan_wikilinks(universe_key)
    }
}
