//! CO-441: typed meta-DB accessors for `workspace_states` (the "Sala" canvas).
//!
//! Moves the raw `conn().query_row/execute` calls out of
//! `content/workspace/routes.rs` into typed methods on `Storage`. The
//! `workspace_states` table lives in the global meta-DB — the state the global
//! `Mutex<Storage>` legitimately guards.

use rusqlite::{Result, params};
use serde::{Deserialize, Serialize};

use super::Storage;

/// A persisted workspace ("Sala") layout for one user in one workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub id: String,
    pub universe_key: String,
    pub workspace_slug: String,
    pub user_id: String,
    pub layout_json: String,
    pub is_public: bool,
    pub share_token: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Column order shared by every `SELECT` below.
const SELECT_COLS: &str = "id, universe_key, workspace_slug, user_id, layout_json, \
                           is_public, share_token, created_at, updated_at";

/// Map a row in `SELECT_COLS` order into a [`WorkspaceState`].
pub(crate) fn row_to_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceState> {
    Ok(WorkspaceState {
        id: row.get(0)?,
        universe_key: row.get(1)?,
        workspace_slug: row.get(2)?,
        user_id: row.get(3)?,
        layout_json: row.get(4)?,
        is_public: row.get::<_, i64>(5)? != 0,
        share_token: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

/// Collapse a single-row query into `Option`, treating "no rows" as `None`.
fn optional(result: rusqlite::Result<WorkspaceState>) -> Result<Option<WorkspaceState>> {
    match result {
        Ok(ws) => Ok(Some(ws)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

impl Storage {
    /// The caller's saved workspace state, if any.
    pub fn workspace_state_for_user(
        &self,
        universe_key: &str,
        workspace_slug: &str,
        user_id: &str,
    ) -> Result<Option<WorkspaceState>> {
        optional(self.conn().query_row(
            &format!(
                "SELECT {SELECT_COLS} FROM workspace_states \
                 WHERE universe_key = ?1 AND workspace_slug = ?2 AND user_id = ?3"
            ),
            params![universe_key, workspace_slug, user_id],
            row_to_state,
        ))
    }

    /// The most-recently-updated public state for a workspace, if any.
    pub fn public_workspace_state(
        &self,
        universe_key: &str,
        workspace_slug: &str,
    ) -> Result<Option<WorkspaceState>> {
        optional(self.conn().query_row(
            &format!(
                "SELECT {SELECT_COLS} FROM workspace_states \
                 WHERE universe_key = ?1 AND workspace_slug = ?2 AND is_public = 1 \
                 ORDER BY updated_at DESC LIMIT 1"
            ),
            params![universe_key, workspace_slug],
            row_to_state,
        ))
    }

    /// Resolve a share token to its workspace state, if any.
    pub fn workspace_state_by_share_token(&self, token: &str) -> Result<Option<WorkspaceState>> {
        optional(self.conn().query_row(
            &format!("SELECT {SELECT_COLS} FROM workspace_states WHERE share_token = ?1"),
            params![token],
            row_to_state,
        ))
    }

    /// Upsert the caller's workspace layout (one state per user per workspace).
    /// `now` is used for both `created_at` (on insert) and `updated_at`.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_workspace_state(
        &self,
        id: &str,
        universe_key: &str,
        workspace_slug: &str,
        user_id: &str,
        layout_json: &str,
        is_public: bool,
        now: &str,
    ) -> Result<usize> {
        let is_public_int: i64 = if is_public { 1 } else { 0 };
        self.conn().execute(
            "INSERT INTO workspace_states \
             (id, universe_key, workspace_slug, user_id, layout_json, is_public, \
              created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) \
             ON CONFLICT (universe_key, workspace_slug, user_id) DO UPDATE SET \
               layout_json = excluded.layout_json, \
               is_public   = excluded.is_public, \
               updated_at  = excluded.updated_at",
            params![
                id,
                universe_key,
                workspace_slug,
                user_id,
                layout_json,
                is_public_int,
                now,
            ],
        )
    }

    /// Set a share token on the caller's existing state. Returns rows updated
    /// (0 when the caller has no saved state yet).
    pub fn set_workspace_share_token(
        &self,
        token: &str,
        now: &str,
        universe_key: &str,
        workspace_slug: &str,
        user_id: &str,
    ) -> Result<usize> {
        self.conn().execute(
            "UPDATE workspace_states \
             SET share_token = ?1, updated_at = ?2 \
             WHERE universe_key = ?3 AND workspace_slug = ?4 AND user_id = ?5",
            params![token, now, universe_key, workspace_slug, user_id],
        )
    }
}
