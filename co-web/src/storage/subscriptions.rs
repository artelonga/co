use chrono::Utc;
use rusqlite::params;

use super::Storage;
use super::schema::parse_datetime;

impl Storage {
    pub fn pin_subscription(
        &mut self,
        user_id: &str,
        universe_key: &str,
        state_path: Option<&str>,
    ) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
        // Upsert: insert if missing, otherwise update pin.
        self.conn.execute(
            "INSERT INTO subscriptions (user_id, universe_key, subscribed_at, pinned_state) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(user_id, universe_key) DO UPDATE SET pinned_state = ?4",
            params![user_id, universe_key, now, state_path],
        )?;
        Ok(())
    }

    /// 1.60.0: read a subscription's pin (None if not subscribed or no pin).
    pub fn get_subscription_pin(&self, user_id: &str, universe_key: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT pinned_state FROM subscriptions WHERE user_id = ?1 AND universe_key = ?2",
                params![user_id, universe_key],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
    }

    /// Check whether a user is subscribed to a universe.
    pub fn is_subscribed(&self, user_id: &str, universe_key: &str) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM subscriptions WHERE user_id = ?1 AND universe_key = ?2",
                params![user_id, universe_key],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    /// List subscribers for a universe.
    pub fn list_universe_subscribers(
        &self,
        universe_key: &str,
    ) -> Vec<crate::models::Subscription> {
        let mut stmt = match self.conn.prepare(
            "SELECT user_id, universe_key, subscribed_at FROM subscriptions \
             WHERE universe_key = ?1 ORDER BY subscribed_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![universe_key], |row| {
            Ok(crate::models::Subscription {
                user_id: row.get(0)?,
                universe_key: row.get(1)?,
                subscribed_at: parse_datetime(&row.get::<_, String>(2)?),
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Search public-subscribable universes by name/description.
    pub fn search_public_universes(&self, query: &str) -> Vec<crate::models::Universe> {
        let pattern = format!("%{}%", query.to_lowercase());
        let mut stmt = match self.conn.prepare(
            "SELECT key, name, description, owner_id, created_at, is_template, is_public, content_count, \
             COALESCE(requires_login, 0), COALESCE(visibility, 'private') \
             FROM universes \
             WHERE visibility = 'public-subscribable' \
             AND (LOWER(name) LIKE ?1 OR LOWER(description) LIKE ?1 OR LOWER(key) LIKE ?1) \
             ORDER BY content_count DESC LIMIT 50",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map(params![pattern], |row| {
            Ok(crate::models::Universe {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                owner_id: row.get(3)?,
                created_at: parse_datetime(&row.get::<_, String>(4)?),
                is_template: row.get::<_, i64>(5)? != 0,
                is_public: row.get::<_, i64>(6)? != 0,
                content_count: row.get::<_, i64>(7).unwrap_or(0),
                requires_login: row.get::<_, i64>(8).unwrap_or(0) != 0,
                visibility: row
                    .get::<_, String>(9)
                    .unwrap_or_else(|_| "public-subscribable".into()),
                // CO-98: search results don't carry parent_key (not load-bearing
                // for the search UX); leave as None for resilience.
                parent_key: None,
                forked_from: None,
                forked_at_op: None,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    // --- CO-49: Deterministic access check ---

    /// Check the access level for a user (or anonymous) on a given universe.
    ///
    /// Implements the 7-step deterministic check from the CO-49 spec:
    /// 1. template → READ for everyone
    /// 2. owner → READ+WRITE
    /// 3. member with write role (owner/admin/editor) → READ+WRITE
    /// 4. member with read role (viewer/member) → READ
    /// 5. subscribed → READ
    /// 6. public-subscribable → metadata only
    /// 7. otherwise → Denied (404)
    pub fn check_universe_access(
        &self,
        user_id: Option<&str>,
        universe_key: &str,
    ) -> crate::models::UniverseAccess {
        use crate::models::UniverseAccess;

        let universe = match self.get_universe(universe_key) {
            Some(u) => u,
            None => return UniverseAccess::Denied,
        };

        // 1. Template / public-static universes are readable by everyone.
        // `public-static` is the visibility for the seeded read-only public
        // universes (template, timeline trio, etc.) — content is curated and
        // never login-gated.
        if universe.visibility == "template" || universe.visibility == "public-static" {
            return UniverseAccess::ReadOnly;
        }

        // 2–5 require a known user_id.
        if let Some(uid) = user_id {
            // 2. Owner gets full access.
            if universe.owner_id == uid {
                return UniverseAccess::ReadWrite;
            }

            // 3–4. Members: check role.
            if let Some(role) = self.universe_member_role(universe_key, uid) {
                let write_roles = ["owner", "admin", "editor"];
                if write_roles.contains(&role.as_str()) {
                    return UniverseAccess::ReadWrite;
                }
                return UniverseAccess::ReadOnly;
            }

            // 5. Subscribed users get read access.
            if self.is_subscribed(uid, universe_key) {
                return UniverseAccess::ReadOnly;
            }
        }

        // 6. Public-subscribable:
        //    - anonymous → MetadataOnly (discovery surface)
        //    - authenticated → ReadOnly (every authed user is admin per 1.45.0,
        //      and "if you can see it you can edit it" — but the writer gate
        //      decides on write. ReadOnly here is enough for the read path.)
        if universe.visibility == "public-subscribable" {
            if user_id.is_some() {
                return UniverseAccess::ReadOnly;
            }
            return UniverseAccess::MetadataOnly;
        }

        // 7. Everything else is denied.
        UniverseAccess::Denied
    }

    // --- Usage gate / content count ---

    /// Return the universe_key for a given project key, or None if not found.
    ///
    /// CO-77: primary lookup is `project_universe_index` in meta.db.
    /// Falls back to scanning open universe connections if not indexed yet.
    pub fn get_project_universe_key(&self, project_key: &str) -> Option<String> {
        let upper = project_key.to_uppercase();

        // Fast path: project_universe_index
        if let Ok(uk) = self.conn.query_row(
            "SELECT universe_key FROM project_universe_index WHERE project_key = ?1",
            params![upper],
            |row| row.get::<_, String>(0),
        ) {
            return Some(uk);
        }

        // Fallback: check meta.db entries (pre-CO-77 rows)
        let path = format!("projects/{}/_project.md", upper);
        self.conn
            .query_row(
                "SELECT universe_key FROM entries WHERE entry_type = 'project' AND path = ?1",
                params![path],
                |row| row.get(0),
            )
            .ok()
    }

    /// Increment content_count for a universe and return the new value.
    pub fn increment_universe_content_count(&mut self, universe_key: &str) -> i64 {
        self.conn
            .execute(
                "UPDATE universes SET content_count = content_count + 1 WHERE key = ?1",
                params![universe_key],
            )
            .ok();
        self.conn
            .query_row(
                "SELECT content_count FROM universes WHERE key = ?1",
                params![universe_key],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Decrement content_count for a universe by `by`, flooring at 0.
    pub fn decrement_universe_content_count(&mut self, universe_key: &str, by: i64) {
        self.conn
            .execute(
                "UPDATE universes SET content_count = MAX(0, content_count - ?1) WHERE key = ?2",
                params![by, universe_key],
            )
            .ok();
    }

    /// CO-80: Sum of content_count across all universes owned by `user_id`.
    /// Used for tier storage quota checks.
    pub fn count_user_entries(&self, user_id: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(content_count), 0) FROM universes \
                 WHERE owner_id = ?1 AND is_template = 0",
                params![user_id],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// CO-80: Count non-template universes owned by `user_id`.
    /// Used for tier universe count quota checks.
    pub fn count_user_universes(&self, user_id: &str) -> i64 {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM universes WHERE owner_id = ?1 AND is_template = 0",
                params![user_id],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Count comments for a specific task.
    pub fn count_task_comments(&self, project_key: &str, task_id: u64) -> i64 {
        let upper = project_key.to_uppercase();
        let like_pattern = format!("projects/{}/comments/{}-%%", upper, task_id);
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE entry_type = 'comment' AND path LIKE ?1",
                params![like_pattern],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Count all tasks and their comments for a project (used for delete_project decrement).
    pub fn count_project_content(&self, project_key: &str) -> i64 {
        let upper = project_key.to_uppercase();
        let tasks: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM entries \
                 WHERE entry_type = 'task' \
                 AND json_extract(frontmatter_json, '$.project') = ?1",
                params![upper],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let comments: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM entries \
                 WHERE entry_type = 'comment' \
                 AND json_extract(frontmatter_json, '$.project') = ?1",
                params![upper],
                |row| row.get(0),
            )
            .unwrap_or(0);
        tasks + comments
    }

    /// Claim an anonymous universe: transfer ownership to a real user.
    /// `anon_id` must match the universe's current owner_id (must start with "anon-").
    pub fn claim_universe(
        &mut self,
        slug: &str,
        user_id: &str,
        anon_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        let universe = self
            .get_universe(slug)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", slug))?;

        if !universe.owner_id.starts_with("anon-") {
            anyhow::bail!("Universe '{}' is not an anonymous universe", slug);
        }
        if universe.owner_id != anon_id {
            anyhow::bail!("Owner cookie does not match universe owner");
        }

        let now_str = Utc::now().to_rfc3339();

        // Transfer ownership
        self.conn.execute(
            "UPDATE universes SET owner_id = ?1 WHERE key = ?2",
            params![user_id, slug],
        )?;

        // Replace anon member with real user
        self.conn.execute(
            "DELETE FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
            params![slug, anon_id],
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES (?1, ?2, 'owner', ?3)",
            params![slug, user_id, now_str],
        )?;

        self.get_universe(slug)
            .ok_or_else(|| anyhow::anyhow!("Universe not found after claim"))
    }

    // --- Universe form config (CO-24) ---

    /// Get the presentation config for a universe (theme, layout, fonts, tokens).
    pub fn get_universe_form_config(&self, key: &str) -> Option<crate::models::UniverseFormConfig> {
        self.conn
            .query_row(
                "SELECT theme_preset, layout, font_headline, font_body, custom_tokens \
                 FROM universes WHERE key = ?1",
                params![key],
                |row| {
                    let tokens_str: Option<String> = row.get(4)?;
                    Ok(crate::models::UniverseFormConfig {
                        theme_preset: row.get::<_, String>(0)?,
                        layout: row.get::<_, String>(1)?,
                        font_headline: row.get(2)?,
                        font_body: row.get(3)?,
                        custom_tokens: tokens_str.and_then(|s| serde_json::from_str(&s).ok()),
                    })
                },
            )
            .ok()
    }

    /// Apply a partial update to the form config and sync `.universo.yaml`.
    pub fn update_universe_form_config(
        &mut self,
        key: &str,
        update: crate::models::UpdateUniverseFormConfig,
    ) -> anyhow::Result<crate::models::UniverseFormConfig> {
        let mut config = self
            .get_universe_form_config(key)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", key))?;

        if let Some(tp) = update.theme_preset {
            config.theme_preset = tp;
        }
        if let Some(l) = update.layout {
            config.layout = l;
        }
        // Empty string clears the font; a value sets it.
        if let Some(fh) = update.font_headline {
            config.font_headline = if fh.is_empty() { None } else { Some(fh) };
        }
        if let Some(fb) = update.font_body {
            config.font_body = if fb.is_empty() { None } else { Some(fb) };
        }
        // An explicit `null` in JSON becomes None here and clears the tokens.
        config.custom_tokens = update.custom_tokens;

        let tokens_str = config.custom_tokens.as_ref().map(|v| v.to_string());

        self.conn.execute(
            "UPDATE universes SET theme_preset = ?1, layout = ?2, \
             font_headline = ?3, font_body = ?4, custom_tokens = ?5 \
             WHERE key = ?6",
            params![
                config.theme_preset,
                config.layout,
                config.font_headline,
                config.font_body,
                tokens_str,
                key,
            ],
        )?;

        // Sync to .universo.yaml (best-effort; never fails the request).
        let _ = self.write_universo_yaml(key, &config);

        Ok(config)
    }

    /// Write form config to `.universo.yaml` at the universe vault root.
    pub(crate) fn write_universo_yaml(
        &self,
        universe_key: &str,
        config: &crate::models::UniverseFormConfig,
    ) -> anyhow::Result<()> {
        let root = self.universe_root(universe_key);
        std::fs::create_dir_all(&root)?;
        let yaml_path = root.join(".universo.yaml");

        let mut yaml = format!(
            "theme_preset: {}\nlayout: {}\n",
            config.theme_preset, config.layout
        );
        if let Some(fh) = &config.font_headline {
            yaml.push_str(&format!("font_headline: {fh}\n"));
        }
        if let Some(fb) = &config.font_body {
            yaml.push_str(&format!("font_body: {fb}\n"));
        }
        if let Some(tokens) = &config.custom_tokens {
            yaml.push_str(&format!("custom_tokens: {tokens}\n"));
        }

        std::fs::write(yaml_path, yaml)?;
        Ok(())
    }

    // --- Check if data exists ---

    pub fn has_data(&self) -> bool {
        // Primary: project_universe_index has rows → projects exist → real data.
        // This is fast and correct for both fresh installs and seeded test DBs.
        let idx: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM project_universe_index", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        if idx > 0 {
            return true;
        }
        // Secondary: user-created universes beyond 'default' and 'template'.
        // Guards the CO-77 first-boot edge case where project_universe_index is
        // momentarily empty on a pre-CO-77 prod DB before
        // maybe_migrate_entries_to_universe_dbs populates it (prod incident 2026-05-01).
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM universes WHERE key NOT IN ('default', 'template')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0i64)
            > 0
    }

    // --- Template universe ---

    /// Returns true if a template universe already exists (seed already ran).
    pub fn template_exists(&self) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM universes WHERE is_template = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }
}
