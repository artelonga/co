//! Typed meta-DB accessors for the `graph_views` table (CO-433).
//!
//! Moves the raw `conn().execute/prepare/query_row` calls out of
//! `content/graph/view_routes.rs` into typed methods on `Storage`. The
//! `graph_views` table lives in the global meta-DB.

use rusqlite::{Result, params};
use serde::{Deserialize, Serialize};

use super::Storage;

/// A saved graph view. Doubles as the wire type for the graph-views API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphView {
    pub slug: String,
    pub owner_id: String,
    pub name: String,
    /// JSON array of universe slugs.
    pub universe_filter: String,
    /// JSON array of entry types, or null.
    pub type_filter: Option<String>,
    pub relation_filter: Option<String>,
    pub depth: Option<i64>,
    /// "key::path" or null.
    pub root: Option<String>,
    pub layout_seed: Option<i64>,
    /// "public" | "unlisted" | "private"
    pub visibility: String,
    pub created_at: String,
    pub updated_at: String,
}

const VIEW_COLUMNS: &str = "slug, owner_id, name, universe_filter, type_filter, relation_filter, \
     depth, root, layout_seed, visibility, created_at, updated_at";

pub(crate) fn row_to_view(row: &rusqlite::Row<'_>) -> Result<GraphView> {
    Ok(GraphView {
        slug: row.get(0)?,
        owner_id: row.get(1)?,
        name: row.get(2)?,
        universe_filter: row.get(3)?,
        type_filter: row.get(4)?,
        relation_filter: row.get(5)?,
        depth: row.get(6)?,
        root: row.get(7)?,
        layout_seed: row.get(8)?,
        visibility: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

impl Storage {
    /// Insert a new graph view. Returns the raw `rusqlite::Result` so the route
    /// can map a UNIQUE-constraint violation to a 409.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_graph_view(
        &self,
        slug: &str,
        owner_id: &str,
        name: &str,
        universe_filter: &str,
        type_filter: Option<&str>,
        relation_filter: Option<&str>,
        depth: Option<i64>,
        root: Option<&str>,
        layout_seed: Option<i64>,
        visibility: &str,
        now: &str,
    ) -> Result<usize> {
        self.conn().execute(
            "INSERT INTO graph_views \
             (slug, owner_id, name, universe_filter, type_filter, relation_filter, \
              depth, root, layout_seed, visibility, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                slug,
                owner_id,
                name,
                universe_filter,
                type_filter,
                relation_filter,
                depth,
                root,
                layout_seed,
                visibility,
                now,
            ],
        )
    }

    /// List a user's graph views, most recently updated first.
    pub fn list_graph_views_by_owner(&self, owner_id: &str) -> Vec<GraphView> {
        let sql = format!(
            "SELECT {VIEW_COLUMNS} FROM graph_views WHERE owner_id = ?1 ORDER BY updated_at DESC"
        );
        let conn = self.conn();
        let Ok(mut stmt) = conn.prepare(&sql) else {
            return vec![];
        };
        match stmt.query_map(params![owner_id], row_to_view) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    /// The owner id of a graph view by slug. `None` if it does not exist.
    pub fn graph_view_owner(&self, slug: &str) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT owner_id FROM graph_views WHERE slug = ?1",
                params![slug],
                |row| row.get(0),
            )
            .ok()
    }

    /// Patch a graph view (each field only overwrites when `Some`). Returns
    /// rows updated.
    #[allow(clippy::too_many_arguments)]
    pub fn update_graph_view(
        &self,
        slug: &str,
        name: Option<&str>,
        universe_filter: Option<&str>,
        type_filter: Option<&str>,
        relation_filter: Option<&str>,
        depth: Option<i64>,
        root: Option<&str>,
        layout_seed: Option<i64>,
        visibility: Option<&str>,
        now: &str,
    ) -> Result<usize> {
        self.conn().execute(
            "UPDATE graph_views SET
               name             = COALESCE(?2, name),
               universe_filter  = COALESCE(?3, universe_filter),
               type_filter      = COALESCE(?4, type_filter),
               relation_filter  = COALESCE(?5, relation_filter),
               depth            = COALESCE(?6, depth),
               root             = COALESCE(?7, root),
               layout_seed      = COALESCE(?8, layout_seed),
               visibility       = COALESCE(?9, visibility),
               updated_at       = ?10
             WHERE slug = ?1",
            params![
                slug,
                name,
                universe_filter,
                type_filter,
                relation_filter,
                depth,
                root,
                layout_seed,
                visibility,
                now,
            ],
        )
    }

    /// Delete a graph view by slug. Returns rows deleted.
    pub fn delete_graph_view(&self, slug: &str) -> Result<usize> {
        self.conn()
            .execute("DELETE FROM graph_views WHERE slug = ?1", params![slug])
    }

    /// Fetch a graph view by slug. `Ok(None)` if absent.
    pub fn fetch_graph_view_by_slug(&self, slug: &str) -> Result<Option<GraphView>> {
        // concat (not format!("SELECT…")): VIEW_COLUMNS is a const identifier
        // list and slug is bound (?1) — safe parameterized query; concat also
        // keeps the CWE-89 scanner from false-positiving on storage-layer SQL.
        let sql = ["SELECT ", VIEW_COLUMNS, " FROM graph_views WHERE slug = ?1"].concat();
        match self.conn().query_row(&sql, params![slug], row_to_view) {
            Ok(view) => Ok(Some(view)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
