use chrono::Utc;
use rusqlite::params;

use crate::entry_index::make_entry;

use super::Storage;
use super::schema::{seed_page_body, seed_page_frontmatter, upsert_entry_row, walkdir};

impl Storage {
    pub fn prune_orphan_universe_dirs(&self) {
        // 2026-05-02 critical fix: the previous implementation iterated ALL
        // top-level dirs under /data/universes/ and deleted any whose name
        // didn't match a `universes.key` row. That was wrong — UniversePool
        // (CO-77) shards per-universe data.db files at:
        //
        //     /data/universes/<2-hex>/<2-hex>/<key>/data.db
        //
        // The 2-hex shard-prefix dirs (e.g. `68`, `b5`, `0e`) are NOT universe
        // keys — they're hash-prefix directories holding multiple per-universe
        // DB files. Deleting them wipes real universe data.
        //
        // This replacement is narrow on purpose: only deletes dirs whose key
        // matches the EXACT list of known-deprecated keys. Wider cleanup is
        // a manual ops task, not an unattended boot-time pass.
        const KNOWN_DEPRECATED_DIRS: &[&str] = &[
            // CO-142 Phase C deletions
            "co-dev",
            "co-experience",
            // Test/anon residue without DB rows
            "prodtest1776629312",
            "anon-test-1775647138",
            "local-6zc952",
            "local-myks0v",
            "u-mruim7",
            "u-wd4zk2",
        ];

        let universes_root = self.data_dir.join("universes");
        let mut pruned = 0usize;
        for key in KNOWN_DEPRECATED_DIRS {
            let path = universes_root.join(key);
            if !path.exists() {
                continue;
            }
            // Defensive: only delete if NO `universes` row holds this key.
            let exists: i64 = self
                .conn
                .query_row(
                    "SELECT 1 FROM universes WHERE key = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if exists != 0 {
                continue;
            }
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!("prune_orphan_universe_dirs: failed to remove {}: {e}", key);
                continue;
            }
            pruned += 1;
            tracing::info!(
                "prune_orphan_universe_dirs: removed deprecated dir '{}'",
                key
            );
        }
        if pruned > 0 {
            tracing::info!("prune_orphan_universe_dirs: pruned {pruned} known-deprecated dir(s)");
        }
    }

    /// Re-ingest entries from filesystem .md files into per-universe `data.db`.
    ///
    /// 2026-05-02 recovery: the previous prune_orphan_universe_dirs deleted
    /// shard-prefix dirs (e.g. /data/universes/68/), which contained per-
    /// universe `data.db` files. The flat .md content survived (lives at
    /// /data/universes/<key>/), but the SQLite shards got recreated empty.
    /// This function walks /data/universes/<key>/**/*.md for every system
    /// universe and upserts each entry into its per-universe DB.
    ///
    /// Idempotent — uses upsert. Safe to run on every boot. Skipped for
    /// universes whose entry count is non-zero (already populated).
    pub fn rebuild_entries_from_filesystem(&mut self, keys: &[&str]) {
        let now_str = Utc::now().to_rfc3339();
        for key in keys {
            let universe_root = self.data_dir.join("universes").join(key);
            if !universe_root.exists() {
                continue;
            }
            // Skip if per-universe DB already has entries (avoids redundant work).
            let already_has: i64 = {
                let uc = self.universe_pool.get_or_open(key);
                let conn = uc.lock().expect("universe conn lock");
                conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
                    .unwrap_or(0)
            };
            if already_has > 0 {
                continue;
            }

            let mut count = 0usize;
            // Recursive walk of the universe's filesystem.
            let walk = walkdir(&universe_root);
            for fs_path in walk {
                let rel = match fs_path.strip_prefix(&universe_root) {
                    Ok(r) => r.to_path_buf(),
                    Err(_) => continue,
                };
                if rel.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }
                let raw = match std::fs::read_to_string(&fs_path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let entry_path = rel.to_string_lossy().to_string();
                let entry = make_entry(
                    &entry_path,
                    seed_page_frontmatter(&raw, &now_str),
                    seed_page_body(&raw),
                );
                let uc = self.universe_pool.get_or_open(key);
                let conn = uc.lock().expect("universe conn lock");
                if upsert_entry_row(&conn, key, &entry).is_ok() {
                    count += 1;
                }
            }
            if count > 0 {
                tracing::info!(
                    "rebuild_entries_from_filesystem: re-ingested {count} entr(ies) for '{key}' from filesystem"
                );
            }
        }
    }

    /// CO-170 Phase B: drop project entries that have **no tasks**.
    ///
    /// Cross-universe project leak (AL in `co`, CW in `artelonga`, etc.) was
    /// caused by past filesystem syncs writing `projects/<KEY>/_project.md`
    /// into the wrong universe directories. Empirically every leaked stub
    /// has zero tasks — the real content (CO-N user-stories in `co`,
    /// `modelos/*.md` content in `artelonga`) lives outside the projects
    /// surface entirely.
    ///
    /// Algorithm: walk every universe's per-universe DB; for each
    /// `entry_type = 'project'` row, count entries under `projects/<KEY>/`
    /// excluding the `_project.md` itself. If zero, drop the project row,
    /// the directory itself, and any `project_universe_index` mapping that
    /// pointed there.
    ///
    /// Idempotent — a re-run after cleanup just no-ops since there are no
    /// more zero-task projects to drop.
    pub fn cleanup_empty_projects(&self) -> usize {
        let universe_keys = self.all_universe_keys();
        let mut total_dropped = 0usize;

        for ukey in &universe_keys {
            let uc = self.universe_pool.get_or_open(ukey);
            let conn = match uc.lock() {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Find project paths in this universe.
            let mut stmt = match conn.prepare(
                "SELECT path, frontmatter_json FROM entries \
                 WHERE universe_key = ?1 AND entry_type = 'project'",
            ) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let projects: Vec<(String, String)> = stmt
                .query_map(params![ukey], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map(|i| i.filter_map(|r| r.ok()).collect())
                .unwrap_or_default();
            drop(stmt);

            for (proj_path, fm_json) in projects {
                // Only consider stubs at `projects/<KEY>/_project.md`.
                let prefix = match proj_path.strip_suffix("/_project.md") {
                    Some(p) => p,
                    None => continue,
                };
                let dir_prefix = format!("{prefix}/");

                // Count siblings under the project dir, excluding _project.md.
                let task_like_count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM entries \
                         WHERE universe_key = ?1 \
                           AND path LIKE ?2 \
                           AND path <> ?3",
                        params![ukey, format!("{dir_prefix}%"), proj_path],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                if task_like_count > 0 {
                    continue;
                }

                // Pluck the project key for index cleanup.
                let proj_key = serde_json::from_str::<serde_json::Value>(&fm_json)
                    .ok()
                    .and_then(|v| v.get("key").and_then(|k| k.as_str()).map(str::to_string));

                let _ = conn.execute(
                    "DELETE FROM entries WHERE universe_key = ?1 AND path = ?2",
                    params![ukey, proj_path],
                );

                if let Some(k) = proj_key.as_deref() {
                    let _ = self.conn.execute(
                        "DELETE FROM project_universe_index \
                         WHERE project_key = ?1 AND universe_key = ?2",
                        params![k, ukey],
                    );
                }

                tracing::info!(
                    "CO-170: dropped empty project {ukey}/{} (key={:?})",
                    prefix.trim_start_matches("projects/"),
                    proj_key
                );
                total_dropped += 1;
            }
        }

        total_dropped
    }

    /// Phase C (CO-142): hard-delete the deprecated `co-dev` and `co-experience`
    /// universe rows and their membership records. Idempotent — DELETE WHERE
    /// is a no-op when the rows are already gone.
    /// CO-170: soft-hide a fixed list of placeholder / pre-merge / empty
    /// universes from sidebar listings. Idempotent — sets `hidden = 1` only
    /// if currently 0, so admins can manually un-hide via SQL. The content
    /// stays accessible via direct URL until a proper migration moves it.
    pub fn hide_deprecated_universes(&mut self) {
        let to_hide: &[&str] = &[
            // Empty placeholder parents that exist only to nest other universes;
            // they confuse users by appearing in the sidebar with 0 entries.
            "language",
            "topologia",
            // Pre-merge mbya corpus. The 1.69.0 changelog said comunicacao
            // would absorb it but the actual content move never landed —
            // see CO-170 Phase C.
            "mbya",
        ];
        for key in to_hide {
            let updated = self
                .conn
                .execute(
                    "UPDATE universes SET hidden = 1 WHERE key = ?1 AND COALESCE(hidden, 0) = 0",
                    params![key],
                )
                .unwrap_or(0);
            if updated > 0 {
                tracing::info!("CO-170: marked '{key}' hidden");
            }
        }
    }

    // CO-170 helper — see `move_entries_strip_prefix`.
}
