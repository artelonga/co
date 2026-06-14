use chrono::Utc;
use rusqlite::params;
use serde_json::json;

use crate::entry_index::{EntryRow, make_entry};

use super::Storage;
use super::schema::{entry_row_from_sql, upsert_entry_row};

use super::{
    MoveRow, SEED_TIMELINE_HUMANITY_INDEX_MD, SEED_TIMELINE_HUMANITY_JSON,
    SEED_TIMELINE_TEMPO_INDEX_MD, SEED_TIMELINE_TEMPO_JSON, SEED_TIMELINE_UNIVERSO_INDEX_MD,
    SEED_TIMELINE_UNIVERSO_JSON,
};

impl Storage {
    pub fn migrate_co170_phase_b(&self) -> usize {
        let mut total = 0usize;

        // Step 1 — drop tutorial / template leakage from `co`.
        for pattern in [
            "data/universes/template/projects/MP/%",
            "data/universes/template/projects/CO/%",
            "data/universes/template/timeline/%",
            "data/universes/template/content/%",
            "data/universes/local-2cw54k/projects/LOCACO/%",
        ] {
            let n = self.delete_entries_matching("co", pattern);
            if n > 0 {
                tracing::info!("CO-170 phase B: dropped {n} entries from co LIKE '{pattern}'");
                total += n;
            }
        }

        // Step 2 — co/data/universes/default/projects/AL/* → artelonga/projects/AL/*.
        let n = self.move_entries_strip_prefix(
            "co",
            "data/universes/default/projects/AL/%",
            "artelonga",
            "data/universes/default/",
        );
        if n > 0 {
            tracing::info!(
                "CO-170 phase B: moved {n} entries co/projects/AL → artelonga/projects/AL"
            );
            total += n;
        }

        // Step 3 — co/data/universes/default/projects/QA/* → quilomboaraucaria/projects/QA/*.
        let n = self.move_entries_strip_prefix(
            "co",
            "data/universes/default/projects/QA/%",
            "quilomboaraucaria",
            "data/universes/default/",
        );
        if n > 0 {
            tracing::info!(
                "CO-170 phase B: moved {n} entries co/projects/QA → quilomboaraucaria/projects/QA"
            );
            total += n;
        }

        // Step 4 — drop empty `co` stubs that overlap with destination keys
        // before we move artelonga's real content into them.
        for key in ["API", "DS", "PLT", "CO"] {
            let pattern = format!("data/universes/default/projects/{key}/%");
            let n = self.delete_entries_matching("co", &pattern);
            if n > 0 {
                tracing::info!("CO-170 phase B: dropped {n} stub entries co LIKE '{pattern}'");
                total += n;
            }
        }

        // Step 5 — artelonga/projects/{API,DS,PLT,CW}/* → co/projects/{...}/*.
        for key in ["API", "DS", "PLT", "CW"] {
            let n = self.move_entries_strip_prefix(
                "artelonga",
                &format!("projects/{key}/%"),
                "co",
                "", // path unchanged on destination
            );
            if n > 0 {
                tracing::info!(
                    "CO-170 phase B: moved {n} entries artelonga/projects/{key} → co/projects/{key}"
                );
                total += n;
            }
        }

        total
    }

    fn delete_entries_matching(&self, universe: &str, like_pattern: &str) -> usize {
        let pool = self.universe_pool.get_or_open(universe);
        let conn = match pool.lock() {
            Ok(c) => c,
            Err(_) => return 0,
        };
        conn.execute(
            "DELETE FROM entries WHERE universe_key = ?1 AND path LIKE ?2",
            params![universe, like_pattern],
        )
        .unwrap_or(0)
    }

    /// Move every entry from `src_universe` matching `src_pattern` into
    /// `dst_universe`, with `strip_prefix` removed from each entry's path.
    /// Returns the number of rows moved.
    #[allow(clippy::too_many_arguments)]
    fn move_entries_strip_prefix(
        &self,
        src_universe: &str,
        src_pattern: &str,
        dst_universe: &str,
        strip_prefix: &str,
    ) -> usize {
        // ---- Phase 1: read src rows
        let rows = {
            let src_pool = self.universe_pool.get_or_open(src_universe);
            let src_conn = match src_pool.lock() {
                Ok(c) => c,
                Err(_) => return 0,
            };
            let mut stmt = match src_conn.prepare(
                "SELECT path, entry_type, title, frontmatter_json, payload, body, body_hash, \
                 created_at, updated_at \
                 FROM entries WHERE universe_key = ?1 AND path LIKE ?2",
            ) {
                Ok(s) => s,
                Err(_) => return 0,
            };
            let rs: Vec<MoveRow> = stmt
                .query_map(params![src_universe, src_pattern], |row| {
                    Ok(MoveRow {
                        path: row.get(0)?,
                        entry_type: row.get(1)?,
                        title: row.get(2)?,
                        frontmatter_json: row.get(3)?,
                        payload: row.get(4)?,
                        body: row.get(5)?,
                        body_hash: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                })
                .map(|i| i.filter_map(|r| r.ok()).collect())
                .unwrap_or_default();
            rs
        };

        if rows.is_empty() {
            return 0;
        }

        // ---- Phase 2: insert into dst with stripped path
        {
            let dst_pool = self.universe_pool.get_or_open(dst_universe);
            let dst_conn = match dst_pool.lock() {
                Ok(c) => c,
                Err(_) => return 0,
            };
            for row in &rows {
                let new_path: &str = if strip_prefix.is_empty() {
                    row.path.as_str()
                } else {
                    row.path.strip_prefix(strip_prefix).unwrap_or(&row.path)
                };
                let _ = dst_conn.execute(
                    "INSERT OR REPLACE INTO entries \
                     (path, universe_key, entry_type, title, frontmatter_json, payload, \
                      body, body_hash, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        new_path,
                        dst_universe,
                        row.entry_type,
                        row.title,
                        row.frontmatter_json,
                        row.payload,
                        row.body,
                        row.body_hash,
                        row.created_at,
                        row.updated_at,
                    ],
                );
            }
        }

        // ---- Phase 3: delete from src
        {
            let src_pool = self.universe_pool.get_or_open(src_universe);
            if let Ok(src_conn) = src_pool.lock() {
                let _ = src_conn.execute(
                    "DELETE FROM entries WHERE universe_key = ?1 AND path LIKE ?2",
                    params![src_universe, src_pattern],
                );
            }
        }

        rows.len()
    }

    /// Rebuild `project_universe_index` from scratch by walking each
    /// universe's project entries. Cheap (<100 rows total). Called after
    /// migrate_co170_phase_b actually moves something.
    ///
    /// Conflict resolution: when two universes share a `key` (e.g.
    /// `co/projects/CO/_project.md` "CO Platform" and
    /// `template/projects/CO/_project.md` "Bem-vindo ao Co" both have
    /// `key: "CO"`), real content universes win over the template /
    /// timeline / hidden ones — they're processed first and `INSERT OR
    /// IGNORE` keeps the first registration. This makes `/api/projects/CO/tasks`
    /// resolve to the platform's CO Platform, not the tutorial.
    pub fn rebuild_project_universe_index(&self) {
        // Defensive — migration v26 created this table but prod boot logs
        // showed `no such table: project_universe_index` on INSERT, meaning
        // the v26 migration block was either skipped (schema_version
        // already >= 26 when the migration was added) or the table was
        // dropped by a since-removed code path. `CREATE TABLE IF NOT EXISTS`
        // is idempotent and cheap; do it before the DELETE so the rebuild
        // can populate a fresh table on every boot if needed.
        let _ = self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS project_universe_index (
                project_key  TEXT PRIMARY KEY,
                universe_key TEXT NOT NULL
            );",
        );
        let _ = self.conn.execute("DELETE FROM project_universe_index", []);

        // Sort keys: non-template/non-system content universes first, then
        // templates/timelines/hidden last. INSERT OR IGNORE means the
        // first-seen registration for any given project_key wins.
        let mut keys = self.all_universe_keys();
        keys.sort_by_key(|k| {
            let is_low_priority = matches!(
                k.as_str(),
                "template" | "tempo" | "humanity" | "universo" | "yggdrasil"
            );
            (is_low_priority, k.clone())
        });

        let mut total_indexed = 0usize;
        for ukey in &keys {
            let pool = self.universe_pool.get_or_open(ukey);
            let conn = match pool.lock() {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut stmt = match conn.prepare(
                "SELECT frontmatter_json FROM entries \
                 WHERE universe_key = ?1 AND entry_type = 'project'",
            ) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let frontmatters: Vec<String> = stmt
                .query_map(params![ukey], |row| row.get::<_, String>(0))
                .map(|i| i.filter_map(|r| r.ok()).collect())
                .unwrap_or_default();
            drop(stmt);
            drop(conn);

            tracing::info!(
                "CO-170 rebuild: universe '{}' has {} project frontmatters",
                ukey,
                frontmatters.len()
            );

            for fm_json in frontmatters {
                let parsed = serde_json::from_str::<serde_json::Value>(&fm_json);
                let key_opt = parsed
                    .as_ref()
                    .ok()
                    .and_then(|fm| fm.get("key"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                if key_opt.is_none() {
                    tracing::warn!(
                        "CO-170 rebuild: universe '{}' project frontmatter missing 'key' or unparseable: {}",
                        ukey,
                        &fm_json[..fm_json.len().min(200)]
                    );
                    continue;
                }
                let key = key_opt.unwrap();
                let upper = key.to_uppercase();
                match self.conn.execute(
                    "INSERT OR IGNORE INTO project_universe_index \
                     (project_key, universe_key) VALUES (?1, ?2)",
                    params![upper, ukey],
                ) {
                    Ok(0) => {
                        tracing::info!(
                            "CO-170 rebuild: skipped {upper} → {} (already indexed by earlier universe)",
                            ukey
                        );
                    }
                    Ok(n) => {
                        tracing::info!("CO-170 rebuild: indexed {upper} → {} ({n} rows)", ukey);
                        total_indexed += 1;
                    }
                    Err(e) => {
                        tracing::warn!("CO-170 rebuild: INSERT failed for {upper} → {}: {e}", ukey);
                    }
                }
            }
        }
        tracing::info!("CO-170 phase B: project_universe_index rebuilt with {total_indexed} rows");
    }

    pub fn delete_deprecated_universes(&mut self) {
        for key in ["co-dev", "co-experience"] {
            let _ = self.conn.execute(
                "DELETE FROM universe_members WHERE universe_key = ?1",
                params![key],
            );
            let deleted = self
                .conn
                .execute("DELETE FROM universes WHERE key = ?1", params![key])
                .unwrap_or(0);
            if deleted > 0 {
                tracing::info!("CO-142: deleted deprecated universe '{key}'");
            }
        }
    }

    /// Phase D (CO-142): hard-delete stale quilombo variant rows that have no
    /// documented purpose and accumulated via manual experiments. Idempotent.
    pub fn delete_stale_quilombo_variants(&mut self) {
        for key in [
            "quilombo-blog",
            "quilombo-blog-2",
            "quilombo-blog-3",
            "qa-dev",
        ] {
            let _ = self.conn.execute(
                "DELETE FROM universe_members WHERE universe_key = ?1",
                params![key],
            );
            let deleted = self
                .conn
                .execute("DELETE FROM universes WHERE key = ?1", params![key])
                .unwrap_or(0);
            if deleted > 0 {
                tracing::info!("CO-142: deleted stale quilombo variant '{key}'");
            }
        }
    }

    /// Phase B (CO-142): recompute `content_count` for every universe by counting
    /// rows in each per-universe entries DB. Runs on every boot so the column stays
    /// accurate even when seed paths bypass `increment_universe_content_count`.
    pub fn recompute_content_counts(&mut self) {
        let keys: Vec<String> = self
            .conn
            .prepare("SELECT key FROM universes")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get(0))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default();

        for key in &keys {
            let count: i64 = {
                let uc = self.universe_pool.get_or_open(key);
                let conn = uc.lock().expect("universe conn lock");
                conn.query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
                    .unwrap_or(0)
            };
            let _ = self.conn.execute(
                "UPDATE universes SET content_count = ?1 WHERE key = ?2",
                params![count, key],
            );
        }
        tracing::info!(
            "CO-142: recomputed content_count for {} universe(s)",
            keys.len()
        );
    }

    /// CO-153: all universe keys from the global metadata DB.
    ///
    /// Used by the cross-universe inbound relation query to know which per-universe
    /// DBs to scan.
    pub fn all_universe_keys(&self) -> Vec<String> {
        self.conn
            .prepare("SELECT key FROM universes")
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get(0))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default()
    }

    /// Returns true if the given timeline universe already exists.
    pub fn timeline_universe_exists(&self, key: &str) -> bool {
        self.get_universe(key).is_some()
    }

    /// Seed a single timeline universe from its JSON manifest + index markdown.
    ///
    /// The manifest defines: universe metadata (`key`, `name`, `description`,
    /// `theme_preset`, `layout`) and an array of `events` each with
    /// `slug`, `title`, `date_year`, and `description`. Events are written as
    /// `type: event` entries under `events/<slug>.md`. The index is written
    /// as `index.md` (rendered as the universe home page).
    ///
    /// Idempotent — `INSERT OR IGNORE` for the universe row, `upsert_entry_row`
    /// for each entry. Safe to re-call on every boot to refresh content.
    pub fn seed_timeline_universe(
        &mut self,
        manifest_json: &str,
        index_md: &str,
    ) -> anyhow::Result<()> {
        let manifest: serde_json::Value = serde_json::from_str(manifest_json)
            .map_err(|e| anyhow::anyhow!("timeline manifest parse: {e}"))?;
        let universe = manifest
            .get("universe")
            .ok_or_else(|| anyhow::anyhow!("timeline manifest missing `universe`"))?;
        let key = universe
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("timeline manifest missing universe.key"))?;
        let name = universe.get("name").and_then(|v| v.as_str()).unwrap_or(key);
        let description = universe
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let theme_preset = universe
            .get("theme_preset")
            .and_then(|v| v.as_str())
            .unwrap_or("modern");
        let layout = universe
            .get("layout")
            .and_then(|v| v.as_str())
            .unwrap_or("timeline");

        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Universe row — public read, no login required, system-owned.
        // parent_key='template' (CO-98) so the timeline trio appears nested
        // under the template universe in the SPA sidebar tree-build.
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public, \
              requires_login, visibility, theme_preset, layout, content_count, parent_key) \
             VALUES (?1, ?2, ?3, 'system', ?4, 0, 1, 0, 'public-static', ?5, ?6, 0, 'template')",
            params![key, name, description, now_str, theme_preset, layout],
        );
        // Existing rows from before CO-98 may have NULL parent_key — backfill
        // idempotently on every boot. Only updates rows that look like the
        // timeline trio (system-owned, public-static), so unrelated existing
        // rows are not touched.
        let _ = self.conn.execute(
            "UPDATE universes SET parent_key = 'template' \
             WHERE key = ?1 AND parent_key IS NULL \
             AND owner_id = 'system' AND visibility = 'public-static'",
            params![key],
        );

        let universe_root = self.universe_root(key);

        // index.md → page entry rendered as the front page by the SPA.
        let index_entry = make_entry(
            "index.md",
            json!({
                "type": "page",
                "slug": "index",
                "title": name,
                "tags": ["timeline", "index"],
                "created": now_str,
                "modified": now_str,
            }),
            index_md,
        );
        if let Err(e) = co::entry::write_entry(&universe_root, &index_entry) {
            tracing::warn!("seed_timeline_universe({key}): write index.md: {e}");
        }
        if let Err(e) = upsert_entry_row(&self.conn, key, &index_entry) {
            tracing::warn!("seed_timeline_universe({key}): upsert index.md: {e}");
        }

        // Each event → `events/<slug>.md` with `type: event`. Body is the
        // description; frontmatter carries `title`, `date_year`.
        let events = manifest
            .get("events")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("timeline manifest missing `events` array"))?;
        let mut count = 0usize;
        for event in events {
            let slug = event
                .get("slug")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("event missing slug"))?;
            let title = event.get("title").and_then(|v| v.as_str()).unwrap_or(slug);
            // `date_year` may exceed i64 range for far-future cosmic events
            // (e.g. heat death ~10^100). Read as f64 for storage; the SPA
            // parses it back as a number on the timeline.
            let date_year = event
                .get("date_year")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| anyhow::anyhow!("event {slug} missing or non-numeric date_year"))?;
            let description = event
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let path = format!("events/{}.md", slug);
            let fm = json!({
                "type": "event",
                "slug": slug,
                "title": title,
                "date_year": date_year,
                "description": description,
                "tags": ["timeline"],
                "created": now_str,
                "modified": now_str,
            });
            let entry = make_entry(&path, fm, description);
            if let Err(e) = co::entry::write_entry(&universe_root, &entry) {
                tracing::warn!("seed_timeline_universe({key}): write {path}: {e}");
            }
            if let Err(e) = upsert_entry_row(&self.conn, key, &entry) {
                tracing::warn!("seed_timeline_universe({key}): upsert {path}: {e}");
            } else {
                count += 1;
            }
        }

        // Update content_count to match what we wrote.
        let _ = self.conn.execute(
            "UPDATE universes SET content_count = ?1 WHERE key = ?2",
            params![(count + 1) as i64, key],
        );

        tracing::info!(
            "Timeline universe '{}' seeded ({} events + index.md)",
            key,
            count
        );
        Ok(())
    }

    /// Seed all three timeline universes (`tempo`, `humanity`, `universo`).
    /// Each is independent — failure of one doesn't stop the others.
    pub fn seed_all_timeline_universes(&mut self) {
        for (label, manifest, index) in [
            (
                "tempo",
                SEED_TIMELINE_TEMPO_JSON,
                SEED_TIMELINE_TEMPO_INDEX_MD,
            ),
            (
                "humanity",
                SEED_TIMELINE_HUMANITY_JSON,
                SEED_TIMELINE_HUMANITY_INDEX_MD,
            ),
            (
                "universo",
                SEED_TIMELINE_UNIVERSO_JSON,
                SEED_TIMELINE_UNIVERSO_INDEX_MD,
            ),
        ] {
            if let Err(e) = self.seed_timeline_universe(manifest, index) {
                tracing::warn!("seed_timeline_universe({label}): {e}");
            }
        }
    }

    /// Returns true if the given project belongs to a template universe.
    pub fn is_project_in_template(&self, project_key: &str) -> bool {
        let universe_key = match self.get_project_universe_key(project_key) {
            Some(k) => k,
            None => return false,
        };

        let v: i64 = self
            .conn
            .query_row(
                "SELECT is_template FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .unwrap_or(0);
        v != 0
    }

    /// List projects for a universe that has `is_public = 1`.
    /// Returns Err if the universe doesn't exist or is not public.
    pub fn list_projects_for_public_universe(
        &self,
        universe_key: &str,
    ) -> anyhow::Result<Vec<crate::models::Project>> {
        let is_public: i64 = self
            .conn
            .query_row(
                "SELECT is_public FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .map_err(|_| anyhow::anyhow!("Universe '{}' not found", universe_key))?;

        if is_public == 0 {
            anyhow::bail!("Universe '{}' is not public", universe_key);
        }

        Ok(self.list_projects_for_universe(universe_key))
    }

    /// Copy entries (projects, tasks, pages) from source into an already-existing target universe.
    /// Returns the number of entries cloned.
    #[allow(dead_code)]
    fn clone_universe_internal(
        &mut self,
        source_key: &str,
        target_key: &str,
        now_str: &str,
    ) -> anyhow::Result<i64> {
        let mut count: i64 = 0;
        let target_root = self.universe_root(target_key);

        // Collect ALL entries from source per-universe DB (CO-77).
        let rows: Vec<EntryRow> = {
            let src_uc = self.universe_pool.get_or_open(source_key);
            let src_guard = src_uc.lock().expect("source universe conn lock");
            let mut stmt = src_guard.prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries WHERE universe_key = ?1",
            )?;
            stmt.query_map(params![source_key], entry_row_from_sql)?
                .filter_map(|r| r.ok())
                .collect()
        };

        let tgt_uc = self.universe_pool.get_or_open(target_key);

        for row in &rows {
            // Derive new project key for project entries
            let mut new_fm = row.frontmatter.clone();
            let mut new_path = row.path.clone();

            if row.entry_type == "project" {
                let old_key = row
                    .frontmatter
                    .get("key")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let new_key = self.derive_unique_project_key(target_key, &old_key);
                new_path = format!("projects/{}/_project.md", new_key);
                if let Some(obj) = new_fm.as_object_mut() {
                    obj.insert("key".to_string(), json!(new_key));
                    obj.insert("created".to_string(), json!(now_str));
                    obj.insert("modified".to_string(), json!(now_str));
                }
                let entry = make_entry(&new_path, new_fm, &row.body);
                let _ = co::entry::write_entry(&target_root, &entry);
                {
                    let tgt_guard = tgt_uc.lock().expect("target universe conn lock");
                    let _ = upsert_entry_row(&tgt_guard, target_key, &entry);
                }
                // Register in routing index so get_project() can find it
                let _ = self.conn.execute(
                    "INSERT OR IGNORE INTO project_universe_index \
                     (project_key, universe_key) VALUES (?1, ?2)",
                    params![new_key, target_key],
                );
                count += 1;

                // Clone tasks for this project
                let old_key2 = old_key.clone();
                let task_rows: Vec<EntryRow> = {
                    let src_uc2 = self.universe_pool.get_or_open(source_key);
                    let src_guard2 = src_uc2.lock().expect("source universe conn lock");
                    let mut stmt2 = src_guard2.prepare(
                        "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                         created_at, updated_at FROM entries \
                         WHERE universe_key = ?1 AND entry_type = 'task' \
                         AND json_extract(frontmatter_json, '$.project') = ?2",
                    )?;
                    stmt2
                        .query_map(params![source_key, old_key2], entry_row_from_sql)?
                        .filter_map(|r| r.ok())
                        .collect()
                };
                for task_row in &task_rows {
                    let task_id = task_row
                        .frontmatter
                        .get("id")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let task_path = format!("projects/{}/{}.md", new_key, task_id);
                    let mut task_fm = task_row.frontmatter.clone();
                    if let Some(obj) = task_fm.as_object_mut() {
                        obj.insert("project".to_string(), json!(new_key));
                        obj.insert("created".to_string(), json!(now_str));
                        obj.insert("modified".to_string(), json!(now_str));
                    }
                    let entry = make_entry(&task_path, task_fm, &task_row.body);
                    let _ = co::entry::write_entry(&target_root, &entry);
                    {
                        let tgt_guard = tgt_uc.lock().expect("target universe conn lock");
                        let _ = upsert_entry_row(&tgt_guard, target_key, &entry);
                    }
                    count += 1;
                }
            } else if row.entry_type == "page" {
                if let Some(obj) = new_fm.as_object_mut() {
                    obj.insert("created".to_string(), json!(now_str));
                    obj.insert("modified".to_string(), json!(now_str));
                }
                let entry = make_entry(&new_path, new_fm, &row.body);
                let _ = co::entry::write_entry(&target_root, &entry);
                {
                    let tgt_guard = tgt_uc.lock().expect("target universe conn lock");
                    let _ = upsert_entry_row(&tgt_guard, target_key, &entry);
                }
                count += 1;
            }
        }

        // Inherit form config
        if let Some(config) = self.get_universe_form_config(source_key) {
            let tokens_str = config.custom_tokens.as_ref().map(|v| v.to_string());
            let _ = self.conn.execute(
                "UPDATE universes SET theme_preset = ?1, layout = ?2 WHERE key = ?3",
                params![config.theme_preset, config.layout, target_key],
            );
            if tokens_str.is_some() {
                let _ = self.conn.execute(
                    "UPDATE universes SET custom_tokens = ?1 WHERE key = ?2",
                    params![tokens_str, target_key],
                );
            }
        }

        Ok(count)
    }

    /// Clone a universe: copy all its projects and tasks into a new universe.
    /// The new universe is NOT a template and is private by default.
    pub fn clone_universe(
        &mut self,
        source_key: &str,
        new_key: &str,
        new_name: &str,
        description: &str,
        owner_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        if self.get_universe(new_key).is_some() {
            anyhow::bail!("Universe '{}' already exists", new_key);
        }
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Create the new universe
        self.conn.execute(
            "INSERT INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
            params![new_key, new_name, description, owner_id, now_str],
        )?;

        // Add owner as member
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members \
             (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, 'owner', ?3)",
            params![new_key, owner_id, now_str],
        )?;

        let mut cloned_entries: i64 = 0;

        // CO-77: read from source per-universe DB, write to destination per-universe DB.
        let src_uc = self.universe_pool.get_or_open(source_key);
        let dst_uc = self.universe_pool.get_or_open(new_key);

        // Collect source project entries
        let source_project_rows: Vec<EntryRow> = {
            let src_guard = src_uc.lock().expect("src universe conn lock");
            let mut stmt = src_guard.prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries \
                 WHERE universe_key = ?1 AND entry_type = 'project'",
            )?;
            stmt.query_map(params![source_key], entry_row_from_sql)?
                .filter_map(|r| r.ok())
                .collect()
        };

        let new_universe_root = self.data_dir.join("universes").join(new_key);

        for proj_row in &source_project_rows {
            let old_pkey = proj_row
                .frontmatter
                .get("key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let new_pkey = self.derive_unique_project_key(new_key, &old_pkey);

            // Build new project entry
            let new_proj_path = format!("projects/{}/_project.md", new_pkey);
            let mut new_proj_fm = proj_row.frontmatter.clone();
            if let Some(obj) = new_proj_fm.as_object_mut() {
                obj.insert("key".to_string(), json!(new_pkey));
                obj.insert("created".to_string(), json!(now_str));
                obj.insert("modified".to_string(), json!(now_str));
            }
            let new_proj_entry = make_entry(&new_proj_path, new_proj_fm, &proj_row.body);
            co::entry::write_entry(&new_universe_root, &new_proj_entry)?;
            {
                let dst_guard = dst_uc.lock().expect("dst universe conn lock");
                upsert_entry_row(&dst_guard, new_key, &new_proj_entry)?;
            }
            // Register project→universe mapping in routing index
            let _ = self.conn.execute(
                "INSERT OR IGNORE INTO project_universe_index (project_key, universe_key) VALUES (?1, ?2)",
                params![new_pkey, new_key],
            );
            cloned_entries += 1;

            // Collect source task entries
            let source_task_rows: Vec<EntryRow> = {
                let src_guard = src_uc.lock().expect("src universe conn lock");
                let mut stmt = src_guard.prepare(
                    "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                     created_at, updated_at FROM entries \
                     WHERE universe_key = ?1 AND entry_type = 'task' \
                     AND json_extract(frontmatter_json, '$.project') = ?2 \
                     AND json_extract(frontmatter_json, '$.archived') = 0",
                )?;
                stmt.query_map(params![source_key, old_pkey], entry_row_from_sql)?
                    .filter_map(|r| r.ok())
                    .collect()
            };

            for task_row in &source_task_rows {
                let task_id = task_row
                    .frontmatter
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let new_task_path = format!("projects/{}/{}.md", new_pkey, task_id);
                let mut new_task_fm = task_row.frontmatter.clone();
                if let Some(obj) = new_task_fm.as_object_mut() {
                    obj.insert("project".to_string(), json!(new_pkey));
                    obj.insert("created".to_string(), json!(now_str));
                    obj.insert("modified".to_string(), json!(now_str));
                }
                let new_task_entry = make_entry(&new_task_path, new_task_fm, &task_row.body);
                co::entry::write_entry(&new_universe_root, &new_task_entry)?;
                {
                    let dst_guard = dst_uc.lock().expect("dst universe conn lock");
                    upsert_entry_row(&dst_guard, new_key, &new_task_entry)?;
                }
                cloned_entries += 1;
            }
        }

        // Clone page entries (content/about, terms, privacy, etc.)
        {
            let source_pages: Vec<EntryRow> = {
                let src_guard = src_uc.lock().expect("src universe conn lock");
                let mut stmt = src_guard.prepare(
                    "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                     created_at, updated_at FROM entries \
                     WHERE universe_key = ?1 AND entry_type = 'page'",
                )?;
                stmt.query_map(params![source_key], entry_row_from_sql)?
                    .filter_map(|r| r.ok())
                    .collect()
            };
            for page_row in &source_pages {
                let mut new_fm = page_row.frontmatter.clone();
                if let Some(obj) = new_fm.as_object_mut() {
                    obj.insert("created".to_string(), json!(now_str));
                    obj.insert("modified".to_string(), json!(now_str));
                }
                let new_page = make_entry(&page_row.path, new_fm, &page_row.body);
                let _ = co::entry::write_entry(&new_universe_root, &new_page);
                {
                    let dst_guard = dst_uc.lock().expect("dst universe conn lock");
                    let _ = upsert_entry_row(&dst_guard, new_key, &new_page);
                }
                cloned_entries += 1;
            }
        }

        // CO-95: copy any remaining entries that weren't picked up by the
        // project/task/page paths above (events, clips, untyped markdown,
        // doc.* generated entries, etc.). Bulk-insert with the new
        // universe_key; preserves path/title/frontmatter/body verbatim so
        // the duplicate is a true snapshot.
        let other_rows: Vec<EntryRow> = {
            let src_guard = src_uc.lock().expect("src universe conn lock");
            let mut stmt = src_guard.prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
                 created_at, updated_at FROM entries \
                 WHERE universe_key = ?1 AND entry_type NOT IN ('project', 'task', 'page')",
            )?;
            stmt.query_map(params![source_key], entry_row_from_sql)?
                .filter_map(|r| r.ok())
                .collect()
        };
        let other_count = other_rows.len() as i64;
        {
            let dst_guard = dst_uc.lock().expect("dst universe conn lock");
            for row in &other_rows {
                let fm_json = serde_json::to_string(&row.frontmatter).unwrap_or_default();
                let _ = dst_guard.execute(
                    "INSERT OR IGNORE INTO entries \
                     (path, universe_key, entry_type, title, frontmatter_json, body, body_hash, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![
                        row.path,
                        new_key,
                        row.entry_type,
                        row.title,
                        fm_json,
                        row.body,
                        row.body_hash,
                        now_str,
                    ],
                );
            }
        }
        cloned_entries += other_count;

        // Set content_count
        self.conn.execute(
            "UPDATE universes SET content_count = ?1 WHERE key = ?2",
            params![cloned_entries, new_key],
        )?;

        // Inherit form config (theme, layout, fonts, tokens) from the source universe.
        if let Some(source_config) = self.get_universe_form_config(source_key) {
            let tokens_str = source_config.custom_tokens.as_ref().map(|v| v.to_string());
            self.conn.execute(
                "UPDATE universes SET theme_preset = ?1, layout = ?2, \
                 font_headline = ?3, font_body = ?4, custom_tokens = ?5 \
                 WHERE key = ?6",
                params![
                    source_config.theme_preset,
                    source_config.layout,
                    source_config.font_headline,
                    source_config.font_body,
                    tokens_str,
                    new_key,
                ],
            )?;
            let _ = self.write_universo_yaml(new_key, &source_config);
        }

        Ok(crate::models::Universe {
            key: new_key.to_string(),
            name: new_name.to_string(),
            description: description.to_string(),
            owner_id: owner_id.to_string(),
            created_at: now,
            is_template: false,
            is_public: false,
            content_count: cloned_entries,
            requires_login: false,
            visibility: "private".into(),
            parent_key: None,
            anon_published_only: false,
            source_kind: None,
            source_url: None,
            source_last_event_at: None,
            source_mode: None,
            surface_dns: None,
        })
    }

    /// Derive a unique project key for a clone, based on the universe key + original project key.
    fn derive_unique_project_key(&self, universe_key: &str, original_key: &str) -> String {
        // Take up to 4 alphanumeric chars from universe key (uppercase) + original key, max 10
        let prefix: String = universe_key
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(4)
            .collect::<String>()
            .to_uppercase();
        let base: String = format!("{}{}", prefix, original_key)
            .chars()
            .take(10)
            .collect();

        // If unique, use as-is; otherwise append a number
        if self.get_project(&base).is_none() {
            return base;
        }
        for i in 2u32..=99 {
            let candidate: String = format!("{}{}", base, i).chars().take(10).collect();
            if self.get_project(&candidate).is_none() {
                return candidate;
            }
        }
        // Fallback: uuid-based suffix (shouldn't happen in practice)
        format!("{}{}", &base[..6.min(base.len())], nanoid::nanoid!(4))
            .chars()
            .take(10)
            .collect()
    }

    // --- List universe keys a user is a member of ---

    pub fn list_user_universes(&self, user_id: &str) -> Vec<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT universe_key FROM universe_members WHERE user_id = ?1")
            .expect("Failed to prepare list_user_universes");
        stmt.query_map(rusqlite::params![user_id], |row| row.get(0))
            .expect("Failed to query user universes")
            .filter_map(|r| r.ok())
            .collect()
    }

    /// List projects the user can see: those in their universes.
    pub fn list_projects_for_user(&self, user_id: &str) -> Vec<crate::models::Project> {
        let universe_keys = self.list_user_universes(user_id);
        let mut result = Vec::new();
        for uk in &universe_keys {
            result.extend(self.list_projects_for_universe(uk));
        }
        result
    }

    // --- Increment project next_id ---

    pub(crate) fn increment_project_next_id(
        &mut self,
        project_key: &str,
        universe_key: &str,
        new_next_id: u64,
    ) {
        let path = format!("projects/{}/_project.md", project_key);
        // CO-77: project entries live in the per-universe data.db, not meta.db.
        let uc = self.universe_pool.get_or_open(universe_key);
        let uc_guard = uc.lock().expect("universe conn lock");
        let result = uc_guard.query_row(
            "SELECT path, universe_key, entry_type, title, frontmatter_json, body, body_hash, \
             created_at, updated_at FROM entries WHERE path = ?1",
            params![path],
            entry_row_from_sql,
        );
        if let Ok(row) = result {
            let mut fm = row.frontmatter.clone();
            if let Some(obj) = fm.as_object_mut() {
                obj.insert("next_id".to_string(), json!(new_next_id));
            }
            let entry = make_entry(&path, fm, &row.body);
            let universe_root = self.universe_root(universe_key);
            let _ = co::entry::write_entry(&universe_root, &entry);
            let _ = upsert_entry_row(&uc_guard, universe_key, &entry);
        }
    }

    /// CO-95 Phase 3 — O(1) universe fork via filesystem copy of the per-universe
    /// SQLite file (data.db).
    ///
    /// Unlike `clone_universe` (which copies entries row-by-row), this method:
    /// 1. WAL-checkpoints the source DB so data.db reflects all committed writes.
    /// 2. Copies data.db with `std::fs::copy` — a single filesystem call.
    /// 3. Reconnects the target DB and bulk-updates `universe_key` in every row.
    /// 4. Clears `entry_events` so the fork starts with a clean op log.
    ///
    /// The caller must ensure the target key does not already exist.
    pub fn fast_fork_universe(
        &mut self,
        source_key: &str,
        new_key: &str,
        new_name: &str,
        description: &str,
        owner_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        if self.get_universe(new_key).is_some() {
            anyhow::bail!("Universe '{}' already exists", new_key);
        }

        let src_db_path = self.universe_pool.db_path(source_key);
        let dst_db_dir = self.universe_pool.universe_dir(new_key);
        let dst_db_path = dst_db_dir.join("data.db");

        std::fs::create_dir_all(&dst_db_dir)?;

        // 1. Checkpoint the source WAL so data.db is self-contained, then copy.
        {
            let src_uc = self.universe_pool.get_or_open(source_key);
            let src_guard = src_uc.lock().expect("fast_fork: source conn lock");
            let _ = src_guard.execute_batch("PRAGMA wal_checkpoint(FULL)");
            std::fs::copy(&src_db_path, &dst_db_path)?;
        }

        // 2. Evict any stale handle, open the new DB, and rewrite universe_key.
        self.universe_pool.evict(new_key);
        {
            let dst_uc = self.universe_pool.get_or_open(new_key);
            let dst_guard = dst_uc.lock().expect("fast_fork: dst conn lock");

            // Bulk-update universe_key in entries (all entries were copied from source).
            dst_guard.execute(
                "UPDATE entries SET universe_key = ?1",
                rusqlite::params![new_key],
            )?;

            // Rebuild FTS index for the new universe_key.
            let _ = dst_guard.execute_batch("DELETE FROM entries_fts");
            let paths_titles_bodies: Vec<(String, Option<String>, String)> = {
                let mut stmt = dst_guard
                    .prepare("SELECT path, title, body FROM entries WHERE universe_key = ?1")?;
                stmt.query_map(rusqlite::params![new_key], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect()
            };
            for (path, title, body) in &paths_titles_bodies {
                let _ = dst_guard.execute(
                    "INSERT INTO entries_fts (universe_key, path, title, body) \
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![new_key, path, title.as_deref().unwrap_or(""), body],
                );
            }

            // Clear op log — fork starts with a fresh history.
            let _ = dst_guard.execute_batch("DELETE FROM entry_events");

            // Reset schema_version sequence to avoid re-running migrations on a
            // already-migrated copy. No-op if the table is already correct.
            // (run_universe_migrations is idempotent via IF NOT EXISTS / version guards.)
        }

        // 3. Create universe metadata row in the global meta.db.
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT INTO universes \
             (key, name, description, owner_id, created_at, is_template, is_public) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
            rusqlite::params![new_key, new_name, description, owner_id, now_str],
        )?;

        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members \
             (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, 'owner', ?3)",
            rusqlite::params![new_key, owner_id, now_str],
        )?;

        // 4. Refresh content_count from the copied entries.
        let content_count: i64 = {
            let dst_uc = self.universe_pool.get_or_open(new_key);
            let dst_guard = dst_uc.lock().expect("fast_fork: count lock");
            dst_guard
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE universe_key = ?1",
                    rusqlite::params![new_key],
                    |row| row.get(0),
                )
                .unwrap_or(0)
        };
        let _ = self.conn.execute(
            "UPDATE universes SET content_count = ?1 WHERE key = ?2",
            rusqlite::params![content_count, new_key],
        );

        // 5. Inherit form config from source.
        if let Some(config) = self.get_universe_form_config(source_key) {
            let tokens_str = config.custom_tokens.as_ref().map(|v| v.to_string());
            let _ = self.conn.execute(
                "UPDATE universes SET theme_preset = ?1, layout = ?2, \
                 font_headline = ?3, font_body = ?4, custom_tokens = ?5 \
                 WHERE key = ?6",
                rusqlite::params![
                    config.theme_preset,
                    config.layout,
                    config.font_headline,
                    config.font_body,
                    tokens_str,
                    new_key,
                ],
            );
            let _ = self.write_universo_yaml(new_key, &config);
        }

        tracing::info!(
            "fast_fork_universe: {source_key} → {new_key} ({content_count} entries copied)"
        );

        Ok(crate::models::Universe {
            key: new_key.to_string(),
            name: new_name.to_string(),
            description: description.to_string(),
            owner_id: owner_id.to_string(),
            created_at: now,
            is_template: false,
            is_public: false,
            content_count,
            requires_login: false,
            visibility: "private".into(),
            parent_key: None,
            anon_published_only: false,
            source_kind: None,
            source_url: None,
            source_last_event_at: None,
            source_mode: None,
            surface_dns: None,
        })
    }
}
