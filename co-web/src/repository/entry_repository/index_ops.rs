//! Index operations on `SqliteEntryRepository` — the full data-access surface
//! the content handlers need (CO-432).
//!
//! The `EntryRepository` trait keeps the small domain-typed CRUD surface from
//! the CO-390 spike; the inherent methods here mirror `EntryIndex` 1:1 (same
//! names, arguments, row types) so handlers stop constructing the index on a
//! raw connection guard without any wire-format change.
//!
//! The `index_entry_create` / `index_entry_update` / `unindex_entry` methods
//! reproduce the combined write blocks of the entry handlers: every projection
//! (entries row, semantic dates, relation edges, references_meta, embeddings)
//! is updated within ONE connection lock scope, exactly like the pre-432
//! handler blocks they replace.

use anyhow::{Result, anyhow};

use super::SqliteEntryRepository;
use crate::entry_index::{EntryEventRow, EntryIndex, EntryRow, TagCount};

impl SqliteEntryRepository {
    // -----------------------------------------------------------------------
    // Reads (1:1 mirrors of EntryIndex)
    // -----------------------------------------------------------------------

    /// Find an entry row by (universe_key, path).
    pub fn get(&self, universe_key: &str, path: &str) -> Result<Option<EntryRow>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).get(universe_key, path)
    }

    /// List entries, optionally filtered by type and frontmatter filter.
    pub fn query(
        &self,
        universe_key: &str,
        entry_type: &str,
        filters: &serde_json::Value,
    ) -> Result<Vec<EntryRow>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).query(universe_key, entry_type, filters)
    }

    /// `query` with an explicit row cap.
    pub fn query_with_limit(
        &self,
        universe_key: &str,
        entry_type: &str,
        filters: &serde_json::Value,
        limit: Option<usize>,
    ) -> Result<Vec<EntryRow>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).query_with_limit(universe_key, entry_type, filters, limit)
    }

    /// Entries whose path starts with `prefix` (CO-264).
    pub fn query_by_path_prefix(
        &self,
        universe_key: &str,
        prefix: &str,
        limit: Option<usize>,
    ) -> Result<Vec<EntryRow>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).query_by_path_prefix(universe_key, prefix, limit)
    }

    /// Execute a compiled query-DSL statement (CO-74).
    pub fn query_raw(
        &self,
        sql: &str,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> Result<Vec<EntryRow>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).query_raw(sql, params)
    }

    /// Full-text search across title + body (FTS5).
    pub fn search(&self, universe_key: &str, query: &str) -> Result<Vec<EntryRow>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).search(universe_key, query)
    }

    /// Top-N entries by `put`-event frequency.
    pub fn popular(&self, universe_key: &str, limit: usize) -> Result<Vec<EntryRow>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).popular(universe_key, limit)
    }

    /// Current body hash for change detection — `None` when the entry is absent.
    pub fn current_body_hash(&self, universe_key: &str, path: &str) -> Option<String> {
        let Ok(conn) = self.lock() else { return None };
        EntryIndex::new(&conn).current_body_hash(universe_key, path)
    }

    /// Most recent `entry_events` rows for a path.
    pub fn list_events_for_path(&self, path: &str, limit: usize) -> Result<Vec<EntryEventRow>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).list_events_for_path(path, limit)
    }

    /// Entries not yet published, review-queue ordered (CO-141).
    pub fn list_review_queue(&self, universe_key: &str) -> Result<Vec<EntryRow>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).list_review_queue(universe_key)
    }

    /// Tag histogram for a universe.
    pub fn tags(&self, universe_key: &str) -> Result<Vec<TagCount>> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).tags(universe_key)
    }

    // -----------------------------------------------------------------------
    // Writes (1:1 mirrors of EntryIndex)
    // -----------------------------------------------------------------------

    /// Upsert an entry row from the storage type (`co::entry::Entry`).
    ///
    /// Named `upsert_entry` because the `EntryRepository` trait already has a
    /// domain-typed `upsert`.
    pub fn upsert_entry(&self, universe_key: &str, entry: &co::entry::Entry) -> Result<()> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).upsert(universe_key, entry)
    }

    /// Upsert preserving sync semantics (vault write path).
    pub fn upsert_synced(&self, universe_key: &str, entry: &co::entry::Entry) -> Result<()> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).upsert_synced(universe_key, entry)
    }

    /// Upsert semantic date rows (CO-73).
    pub fn upsert_dates(
        &self,
        universe_key: &str,
        entry: &co::entry::Entry,
        manifest: Option<&co::manifest::Manifest>,
    ) -> Result<()> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).upsert_dates(universe_key, entry, manifest)
    }

    /// Remove an entry row (+ FTS row).
    pub fn remove(&self, universe_key: &str, path: &str) -> Result<()> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).remove(universe_key, path)
    }

    /// Remove semantic date rows (CO-73).
    pub fn remove_dates(&self, universe_key: &str, path: &str) -> Result<()> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).remove_dates(universe_key, path)
    }

    /// Set review-workflow fields (CO-141).
    #[allow(clippy::too_many_arguments)]
    pub fn set_review_fields(
        &self,
        universe_key: &str,
        path: &str,
        review_status: &str,
        submitted_by: Option<&str>,
        submitted_at: Option<&str>,
        reviewed_by: Option<&str>,
        reviewed_at: Option<&str>,
    ) -> Result<()> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).set_review_fields(
            universe_key,
            path,
            review_status,
            submitted_by,
            submitted_at,
            reviewed_by,
            reviewed_at,
        )
    }

    /// Append a row to the `entry_events` transaction log.
    #[allow(clippy::too_many_arguments)]
    pub fn log_event(
        &self,
        path: &str,
        op: &str,
        body_hash: Option<&str>,
        prev_body_hash: Option<&str>,
        body: Option<&str>,
        frontmatter_json: Option<&str>,
        author_id: Option<&str>,
        request_id: Option<&str>,
    ) -> Result<i64> {
        let conn = self.lock()?;
        EntryIndex::new(&conn).log_event(
            path,
            op,
            body_hash,
            prev_body_hash,
            body,
            frontmatter_json,
            author_id,
            request_id,
        )
    }

    // -----------------------------------------------------------------------
    // Combined write operations (one lock scope, pre-432 handler semantics)
    // -----------------------------------------------------------------------

    /// Index a freshly created entry: entries row + semantic dates + typed FK /
    /// wikilink relations + references_meta sync, in one lock scope.
    ///
    /// Returns the number of relation edges written (for telemetry).
    /// Mirrors the create block of `create_entry`.
    #[allow(clippy::too_many_arguments)]
    pub fn index_entry_create(
        &self,
        universe_key: &str,
        entry: &co::entry::Entry,
        frontmatter: &serde_json::Value,
        body: &str,
        title: Option<&str>,
        manifest: Option<&co::manifest::Manifest>,
        universe_root: &std::path::Path,
    ) -> Result<usize> {
        let conn = self.lock()?;
        let index = EntryIndex::new(&conn);
        index.upsert(universe_key, entry)?;
        // CO-73: index semantic date fields
        index.upsert_dates(universe_key, entry, manifest)?;
        // CO-74/CO-363: extract typed FK + body wikilink relations
        let relation_count = crate::relation_index::sync_entry_relations(
            &conn,
            universe_key,
            &entry.path,
            &entry.entry_type,
            frontmatter,
            body,
            manifest,
        )
        .unwrap_or(0);
        // CO-156: sync references_meta shadow table for reference cards
        crate::content::references::meta::maybe_sync_reference_meta(
            &conn,
            universe_key,
            &entry.path,
            &entry.entry_type,
            frontmatter,
            body,
            title,
            universe_root,
        );
        Ok(relation_count)
    }

    /// Re-index an updated entry: prev-hash capture + entries row +
    /// `entry_events` log + semantic dates + relations + references_meta,
    /// in one lock scope. Mirrors the update block of `update_entry`.
    #[allow(clippy::too_many_arguments)]
    pub fn index_entry_update(
        &self,
        universe_key: &str,
        path: &str,
        entry: &co::entry::Entry,
        frontmatter: &serde_json::Value,
        body: &str,
        manifest: Option<&co::manifest::Manifest>,
        universe_root: &std::path::Path,
    ) -> Result<()> {
        let conn = self.lock()?;
        let index = EntryIndex::new(&conn);
        // 2.7.25: capture prev body_hash for the event log.
        let prev_hash = index.current_body_hash(universe_key, path);
        index.upsert(universe_key, entry)?;
        // 2.7.25: append to entry_events. See vault_routes::write_vault_entry
        // for the parallel hook on the other write path.
        if let Ok(fm_json) = serde_json::to_string(frontmatter)
            && let Err(e) = index.log_event(
                path,
                "put",
                Some(&entry.body_hash),
                prev_hash.as_deref(),
                Some(body),
                Some(&fm_json),
                None,
                None,
            )
        {
            tracing::warn!("entry_events log failed for {universe_key}/{path}: {e}");
        }
        // CO-73: index semantic date fields
        index.upsert_dates(universe_key, entry, manifest)?;
        // CO-74/CO-363: re-sync typed FK + body wikilink relations
        let _ = crate::relation_index::sync_entry_relations(
            &conn,
            universe_key,
            path,
            &entry.entry_type,
            frontmatter,
            body,
            manifest,
        );
        // CO-156: sync references_meta for reference cards
        crate::content::references::meta::maybe_sync_reference_meta(
            &conn,
            universe_key,
            path,
            &entry.entry_type,
            frontmatter,
            body,
            frontmatter.get("title").and_then(|v| v.as_str()),
            universe_root,
        );
        Ok(())
    }

    /// Index a vault write: entries upsert + `entry_events` log inside one
    /// `BEGIN IMMEDIATE` transaction (CO-95 Phase 2 — a crash between the two
    /// writes must never leave them out of sync), then semantic dates,
    /// relations, and references_meta — all in one lock scope.
    ///
    /// Returns the post-write `COUNT(*)` of the universe's entries (used by
    /// the 1.67.0 content_count refresh), or `None` when the count query
    /// fails. Mirrors the write block of `write_vault_entry`.
    pub fn index_vault_write(
        &self,
        universe_key: &str,
        entry: &co::entry::Entry,
        frontmatter: &serde_json::Value,
        body: &str,
        manifest: Option<&co::manifest::Manifest>,
        universe_root: &std::path::Path,
    ) -> Result<Option<i64>> {
        let conn = self.lock()?;
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| anyhow!("begin tx: {e}"))?;
        let prev_hash = EntryIndex::new(&conn).current_body_hash(universe_key, &entry.path);
        let tx_result: Result<()> = {
            let index = EntryIndex::new(&conn);
            index.upsert_synced(universe_key, entry).and_then(|()| {
                if let Ok(fm_json) = serde_json::to_string(&entry.frontmatter) {
                    index
                        .log_event(
                            &entry.path,
                            "put",
                            Some(&entry.body_hash),
                            prev_hash.as_deref(),
                            Some(body),
                            Some(&fm_json),
                            None,
                            None,
                        )
                        .map_err(|e| anyhow!("entry_events log: {e}"))?;
                }
                Ok(())
            })
        };
        match tx_result {
            Ok(()) => conn
                .execute_batch("COMMIT")
                .map_err(|e| anyhow!("commit tx: {e}"))?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        let index = EntryIndex::new(&conn);
        // CO-73: index semantic date fields
        index.upsert_dates(universe_key, entry, manifest)?;
        // CO-74/CO-363: extract typed FK + body wikilink relations
        let _ = crate::relation_index::sync_entry_relations(
            &conn,
            universe_key,
            &entry.path,
            &entry.entry_type,
            frontmatter,
            body,
            manifest,
        );
        // CO-156: sync references_meta for reference cards
        crate::content::references::meta::maybe_sync_reference_meta(
            &conn,
            universe_key,
            &entry.path,
            &entry.entry_type,
            frontmatter,
            body,
            frontmatter.get("title").and_then(|v| v.as_str()),
            universe_root,
        );
        // 1.67.0: report the actual entry count so the caller can refresh
        // the universe's content_count idempotently.
        let actual_count: Option<i64> = conn
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1",
                rusqlite::params![universe_key],
                |row| row.get(0),
            )
            .ok();
        Ok(actual_count)
    }

    /// Unindex a vault delete: entries remove + `entry_events` delete log
    /// inside one `BEGIN IMMEDIATE` transaction (CO-95 Phase 2), then
    /// relation edges and references_meta — one lock scope. Unlike
    /// `unindex_entry`, the vault delete path does not touch semantic dates
    /// or embeddings (pre-432 behavior, preserved).
    pub fn unindex_vault_entry(&self, universe_key: &str, path: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| anyhow!("begin del tx: {e}"))?;
        let prev_hash = EntryIndex::new(&conn).current_body_hash(universe_key, path);
        let del_result: Result<()> = {
            let index = EntryIndex::new(&conn);
            index.remove(universe_key, path).and_then(|()| {
                index
                    .log_event(
                        path,
                        "delete",
                        None,
                        prev_hash.as_deref(),
                        None,
                        None,
                        None,
                        None,
                    )
                    .map(|_| ())
                    .map_err(|e| anyhow!("entry_events del log: {e}"))
            })
        };
        match del_result {
            Ok(()) => conn
                .execute_batch("COMMIT")
                .map_err(|e| anyhow!("commit del tx: {e}"))?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        // CO-74: remove outbound FK relations
        let _ =
            crate::relation_index::RelationIndex::new(&conn).delete_for_entry(universe_key, path);
        // CO-156: remove references_meta (idempotent)
        crate::content::references::meta::remove_reference_meta(&conn, universe_key, path);
        Ok(())
    }

    /// Remove every projection of a deleted entry: entries row + semantic
    /// dates + outbound relations + references_meta + embedding row, in one
    /// lock scope. Mirrors the delete block of `delete_entry`.
    pub fn unindex_entry(&self, universe_key: &str, path: &str) -> Result<()> {
        let conn = self.lock()?;
        let index = EntryIndex::new(&conn);
        index.remove(universe_key, path)?;
        // CO-73: remove semantic date rows
        index.remove_dates(universe_key, path)?;
        // CO-74: remove outbound FK relations
        let _ =
            crate::relation_index::RelationIndex::new(&conn).delete_for_entry(universe_key, path);
        // CO-156: remove references_meta + references_fts (idempotent)
        crate::content::references::meta::remove_reference_meta(&conn, universe_key, path);
        // CO-164: remove embedding row
        let _ = crate::embedding_index::EmbeddingIndex::new(&conn).remove(universe_key, path);
        Ok(())
    }
}
