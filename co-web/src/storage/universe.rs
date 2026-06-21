use chrono::Utc;
use rusqlite::params;
use serde_json::json;

use crate::entry_index::make_entry;

use super::Storage;
use super::schema::{parse_datetime, upsert_entry_row};

impl Storage {
    /// CO-465: ensure a fresh user has their own **private personal universe**
    /// (key = sanitised email local-part, collision-suffixed). Idempotent — a no-op
    /// when the user already owns any universe. Seeds a welcome entry so the
    /// universe isn't empty. Returns the key when one is created.
    ///
    /// Best-effort by contract: callers on the signup/login path must log + continue
    /// on `Err` (a failure here must never block authentication).
    pub fn ensure_personal_universe(
        &mut self,
        user_id: &str,
        email: &str,
        display_name: &str,
    ) -> anyhow::Result<Option<String>> {
        if !self.list_owned_universes(user_id).is_empty() {
            return Ok(None); // already has a universe — leave it alone
        }
        let base: String = email
            .split('@')
            .next()
            .unwrap_or("user")
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .chars()
            .take(40)
            .collect();
        let base = if base.is_empty() {
            "user".to_string()
        } else {
            base
        };
        // Collision-suffix against existing universe keys.
        let mut key = base.clone();
        let mut n = 1;
        while self.get_universe(&key).is_some() {
            n += 1;
            if n > 50 {
                return Ok(None);
            }
            key = format!("{base}-{n}");
        }
        let name = if display_name.trim().is_empty() {
            key.clone()
        } else {
            display_name.to_string()
        };
        self.create_universe(
            crate::models::CreateUniverse {
                key: key.clone(),
                name,
                description: "Seu universo pessoal — privado por padrão.".to_string(),
            },
            user_id,
        )?;
        // Seed a welcome page so the personal universe opens to something.
        let now_str = Utc::now().to_rfc3339();
        let fm = json!({
            "type": "page",
            "title": "Bem-vindo",
            "created": now_str,
            "modified": now_str,
            "tags": []
        });
        let body = format!(
            "# Bem-vindo, {}\n\nEste é o **seu universo pessoal** — privado por padrão, \
             só você vê. Crie tarefas e notas, edite no navegador ou sincronize local \
             com `co sync`. Para descobrir universos públicos e assinar, veja a busca de \
             universos.\n",
            if display_name.trim().is_empty() {
                "👋"
            } else {
                display_name
            }
        );
        let entry = make_entry("content/bem-vindo.md", fm, &body);
        let root = self.universe_root(&key);
        let _ = co::entry::write_entry(&root, &entry);
        let _ = upsert_entry_row(&self.conn, &key, &entry);
        Ok(Some(key))
    }

    pub fn create_universe(
        &mut self,
        create: crate::models::CreateUniverse,
        owner_id: &str,
    ) -> anyhow::Result<crate::models::Universe> {
        if self.get_universe(&create.key).is_some() {
            anyhow::bail!("Universe '{}' already exists", create.key);
        }
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT INTO universes (key, name, description, owner_id, created_at, visibility) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'private')",
            params![
                create.key,
                create.name,
                create.description,
                owner_id,
                now_str
            ],
        )?;
        // Owner is automatically a member
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) VALUES (?1, ?2, 'owner', ?3)",
            params![create.key, owner_id, now_str],
        )?;

        // Create ONE empty project named "Co" — private boards start empty.
        // Project key is unique per universe to avoid cross-universe task leaks.
        let proj_key = format!(
            "{}P",
            create
                .key
                .to_uppercase()
                .chars()
                .take(4)
                .collect::<String>()
        );
        let proj_path = format!("projects/{}/_project.md", proj_key);
        let proj_fm = json!({
            "type": "project",
            "key": proj_key,
            "title": "Bem-vindo ao Co",
            "status": "active",
            "next_id": 1,
            "created": now_str,
            "modified": now_str,
            "archived": false,
            "tags": []
        });
        let proj_entry = make_entry(&proj_path, proj_fm, "");
        let universe_root = self.universe_root(&create.key);
        let _ = co::entry::write_entry(&universe_root, &proj_entry);
        let _ = upsert_entry_row(&self.conn, &create.key, &proj_entry);

        let _ = self.conn.execute(
            "UPDATE universes SET content_count = 1 WHERE key = ?1",
            params![create.key],
        );

        // CO-193: seed a `general` chat room for every new universe.
        if let Err(e) = self.ensure_default_room(&create.key) {
            tracing::warn!(universe_key = %create.key, "ensure_default_room on create: {e}");
        }

        Ok(crate::models::Universe {
            key: create.key,
            name: create.name,
            description: create.description,
            owner_id: owner_id.to_string(),
            created_at: now,
            is_template: false,
            is_public: false,
            content_count: 1,
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

    pub fn get_universe(&self, key: &str) -> Option<crate::models::Universe> {
        // CO-98 hardening: query the stable schema first (always present at
        // schema_v ≥ 17), then opportunistically fetch `parent_key` in a
        // second query. If migration v22 never landed on this DB (or the
        // column is otherwise absent), the second query returns None and the
        // function still produces a valid Universe — instead of returning
        // 404 to the caller as if the universe didn't exist at all.
        let mut universe = self
            .conn
            .query_row(
                "SELECT key, name, description, owner_id, created_at, is_template, is_public, content_count, \
                 COALESCE(requires_login, 0), COALESCE(visibility, 'private'), \
                 COALESCE(anon_published_only, 0) \
                 FROM universes WHERE key = ?1",
                params![key],
                |row| {
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
                        visibility: row.get::<_, String>(9).unwrap_or_else(|_| "private".into()),
                        parent_key: None,
                        anon_published_only: row.get::<_, i64>(10).unwrap_or(0) != 0,
                        source_kind: None,
                        source_url: None,
                        source_last_event_at: None,
                        source_mode: None,
                        surface_dns: None,
                    })
                },
            )
            .ok()?;
        universe.parent_key = self
            .conn
            .query_row(
                "SELECT parent_key FROM universes WHERE key = ?1",
                params![key],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        // CO-383: source attribution fields (v68 columns, may be NULL on pre-migration rows).
        if let Ok((sk, su, sla)) = self.conn.query_row(
            "SELECT source_kind, source_url, source_last_event_at FROM universes WHERE key = ?1",
            params![key],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        ) {
            universe.source_kind = sk;
            universe.source_url = su;
            universe.source_last_event_at = sla;
        }
        // CO-413: source_mode (v83 column). Queried separately from the v68
        // columns so a pre-v83 DB (column absent) degrades to `None` — read-only
        // — WITHOUT also nulling source_kind/source_url and accidentally opening
        // the write gate on yggdrasil.
        universe.source_mode = self
            .conn
            .query_row(
                "SELECT source_mode FROM universes WHERE key = ?1",
                params![key],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        // CO-338: surface_dns (v84 column). Queried separately so a pre-v84 DB
        // (column absent) degrades to `None` without affecting the other fields.
        universe.surface_dns = self
            .conn
            .query_row(
                "SELECT surface_dns FROM universes WHERE key = ?1",
                params![key],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        Some(universe)
    }

    /// CO-338: every universe's `(key, parent_key, surface_dns)` — the registry
    /// the `core::surface` resolver walks to turn a `key::path` reference into a
    /// live deployment URL, and the deployment-snapshot worker reads instead of
    /// a hardcoded unit list. Panic-free under the storage mutex: any SQLite
    /// error degrades to an empty list (see `feedback_no_panic_under_mutex`).
    pub fn list_surface_nodes(&self) -> Vec<co::surface::SurfaceNode> {
        let mut stmt = match self.conn.prepare(
            "SELECT key, parent_key, surface_dns FROM universes \
             WHERE deleted_at IS NULL AND archived_at IS NULL",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("list_surface_nodes prepare: {e}");
                return Vec::new();
            }
        };
        match stmt.query_map([], |row| {
            let key: String = row.get(0)?;
            let parent: Option<String> = row.get(1).unwrap_or(None);
            let surface_dns: Option<String> = row.get(2).unwrap_or(None);
            Ok(co::surface::SurfaceNode {
                key,
                parent,
                surface_dns,
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                tracing::error!("list_surface_nodes query: {e}");
                Vec::new()
            }
        }
    }

    /// CO-96: true when the universe is soft-deleted or archived (i.e. in the
    /// trash). `get_universe` deliberately keeps returning these rows so the
    /// restore flow can find them by key, so public read handlers call this to
    /// present a trashed universe as gone (404) rather than leaking its info.
    pub fn is_universe_trashed(&self, key: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM universes \
                 WHERE key = ?1 AND (deleted_at IS NOT NULL OR archived_at IS NOT NULL)",
                params![key],
                |_| Ok(()),
            )
            .is_ok()
    }

    /// CO-98: list the direct children of a universe (universes whose
    /// `parent_key` equals `parent`). Returns a flat list — callers build the
    /// tree client-side. Deleted/archived children are excluded. Panic-free.
    pub fn list_children_of_universe(&self, parent: &str) -> Vec<crate::models::Universe> {
        let mut stmt = match self.conn.prepare(
            "SELECT key, name, description, owner_id, created_at, is_template, is_public, \
             COALESCE(content_count, 0), COALESCE(requires_login, 0), \
             COALESCE(visibility, 'private') \
             FROM universes \
             WHERE parent_key = ?1 \
               AND deleted_at IS NULL AND archived_at IS NULL \
             ORDER BY name ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("list_children_of_universe prepare: {e}");
                return Vec::new();
            }
        };
        match stmt.query_map(params![parent], |row| {
            Ok(crate::models::Universe {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2).unwrap_or_default(),
                owner_id: row.get(3).unwrap_or_default(),
                created_at: row
                    .get::<_, String>(4)
                    .ok()
                    .and_then(|s| {
                        chrono::DateTime::parse_from_rfc3339(&s)
                            .ok()
                            .map(|d| d.with_timezone(&chrono::Utc))
                    })
                    .unwrap_or_else(chrono::Utc::now),
                is_template: row.get::<_, i64>(5).unwrap_or(0) != 0,
                is_public: row.get::<_, i64>(6).unwrap_or(0) != 0,
                content_count: row.get(7)?,
                requires_login: row.get::<_, i64>(8).unwrap_or(0) != 0,
                visibility: row.get(9)?,
                parent_key: Some(parent.to_owned()),
                anon_published_only: false,
                source_kind: None,
                source_url: None,
                source_last_event_at: None,
                source_mode: None,
                surface_dns: None,
            })
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                tracing::error!("list_children_of_universe query: {e}");
                Vec::new()
            }
        }
    }

    /// CO-383: stamp the last-received event timestamp for an event-bus universe.
    pub fn touch_universe_last_event_at(&self, key: &str, ts: &str) {
        if let Err(e) = self.conn.execute(
            "UPDATE universes SET source_last_event_at = ?1 WHERE key = ?2",
            params![ts, key],
        ) {
            tracing::warn!(universe_key = %key, error = %e, "touch_universe_last_event_at failed");
        }
    }

    /// CO-413: stamp the `source_marker` diagnostic column on a single entry in
    /// a per-universe DB, recording which channel last wrote it. Used after a CO
    /// API/editor write to a bidirectional event-bus universe (`'co-edit'`) so
    /// the row is distinguishable from a `'yggdrasil-live'` inbound write. Best
    /// effort — a failure is logged, never propagated (the write already
    /// succeeded; the marker is observability only).
    pub fn stamp_entry_source_marker(&self, universe_key: &str, path: &str, marker: &str) {
        let uc = self.universe_conn(universe_key);
        match uc.lock() {
            Ok(conn) => {
                if let Err(e) = conn.execute(
                    "UPDATE entries SET source_marker = ?1 \
                     WHERE universe_key = ?2 AND path = ?3",
                    params![marker, universe_key, path],
                ) {
                    tracing::warn!(
                        universe_key = %universe_key, path = %path, error = %e,
                        "stamp_entry_source_marker failed"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    universe_key = %universe_key, path = %path, error = %e,
                    "stamp_entry_source_marker: universe conn poisoned"
                );
            }
        }
    }

    /// CO-330 + CO-337: update runtime universe→repo binding fields (owner-only, all fields optional).
    pub fn update_universe_source(
        &self,
        key: &str,
        local_repo_path: Option<&str>,
        content_subdirs: Option<&str>,
        anon_published_only: Option<bool>,
        remote_url: Option<&str>,
        remote_ref: Option<&str>,
    ) -> rusqlite::Result<()> {
        if let Some(path) = local_repo_path {
            self.conn.execute(
                "UPDATE universes SET local_repo_path = ?1 WHERE key = ?2",
                rusqlite::params![path, key],
            )?;
        }
        if let Some(subdirs) = content_subdirs {
            self.conn.execute(
                "UPDATE universes SET content_subdirs = ?1 WHERE key = ?2",
                rusqlite::params![subdirs, key],
            )?;
        }
        if let Some(published_only) = anon_published_only {
            let v: i64 = if published_only { 1 } else { 0 };
            self.conn.execute(
                "UPDATE universes SET anon_published_only = ?1 WHERE key = ?2",
                rusqlite::params![v, key],
            )?;
        }
        if let Some(url) = remote_url {
            self.conn.execute(
                "UPDATE universes SET remote_url = ?1 WHERE key = ?2",
                rusqlite::params![url, key],
            )?;
        }
        if let Some(r) = remote_ref {
            self.conn.execute(
                "UPDATE universes SET remote_ref = ?1 WHERE key = ?2",
                rusqlite::params![r, key],
            )?;
        }
        Ok(())
    }

    pub fn list_universes_for_user(&self, user_id: &str) -> Vec<crate::models::Universe> {
        // Owned + member + subscribed universes, deduplicated.
        // Also include `owner_id = user_id` directly as a defensive fallback —
        // `create_universe` inserts an owner row in `universe_members`, but
        // historic data or a partial migration could leave that row missing,
        // which would silently hide the user's own universe from their sidebar.
        let mut stmt = self
            .conn
            .prepare(
                "SELECT u.key, u.name, u.description, u.owner_id, u.created_at, \
                 u.is_template, u.is_public, u.content_count, \
                 COALESCE(u.requires_login, 0), COALESCE(u.visibility, 'private') \
                 FROM universes u \
                 WHERE COALESCE(u.hidden, 0) = 0 \
                   AND u.deleted_at IS NULL AND u.archived_at IS NULL \
                   AND ( \
                       u.owner_id = ?1 \
                       OR u.key IN ( \
                         SELECT universe_key FROM universe_members WHERE user_id = ?1 \
                         UNION \
                         SELECT universe_key FROM subscriptions WHERE user_id = ?1 \
                       ) \
                   ) \
                 ORDER BY u.created_at ASC",
            )
            .expect("Failed to prepare list_universes_for_user");
        // CO-98 hardening: same two-query split as get_universe — the bulk
        // SELECT stays on the stable schema, then we opportunistically fetch
        // parent_key per row. Slightly more SQL round-trips but resilient to
        // a partially-applied migration.
        let universes: Vec<crate::models::Universe> = stmt
            .query_map(params![user_id], |row| {
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
                    visibility: row.get::<_, String>(9).unwrap_or_else(|_| "private".into()),
                    parent_key: None,
                    anon_published_only: false,
                    source_kind: None,
                    source_url: None,
                    source_last_event_at: None,
                    source_mode: None,
                    surface_dns: None,
                })
            })
            .expect("Failed to list universes for user")
            .filter_map(|r| r.ok())
            .collect();
        universes
            .into_iter()
            .map(|mut u| {
                u.parent_key = self
                    .conn
                    .query_row(
                        "SELECT parent_key FROM universes WHERE key = ?1",
                        params![u.key],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten();
                u
            })
            .collect()
    }

    /// CO-173: list every universe the user has any relation to (owner / member /
    /// subscriber), each with a metadata bag pulled from the source-of-truth for
    /// that universe. Quilombo metadata is folded in via `quilombo_usuarios.linked_co_user_id`.
    ///
    /// Cross-deployment universes are out of scope until CO-172v2 lands an API mesh.
    pub fn list_universes_with_metadata_for_user(
        &self,
        user_id: &str,
    ) -> Vec<crate::models::UserUniverseEntry> {
        // 1. Universe membership/role (owner row reflected via owner_id check).
        let universes = self.list_universes_for_user(user_id);

        // 2. Pre-fetch the user's quilombo profile (if any) keyed by linked_co_user_id.
        //    The columns we surface in metadata: papel, bio, foto_url, telefone, email.
        let quilombo_meta: Option<serde_json::Value> = self
            .conn
            .query_row(
                "SELECT papel, bio, foto_url, telefone, email \
                 FROM quilombo_usuarios WHERE linked_co_user_id = ?1",
                params![user_id],
                |row| {
                    let papel: String = row.get::<_, String>(0).unwrap_or_default();
                    let bio: Option<String> = row.get(1).ok();
                    let foto_url: Option<String> = row.get(2).ok();
                    let telefone: Option<String> = row.get(3).ok();
                    let email: Option<String> = row.get(4).ok();
                    Ok(serde_json::json!({
                        "papel": papel,
                        "bio": bio,
                        "foto_url": foto_url,
                        "telefone": telefone,
                        "email": email,
                    }))
                },
            )
            .ok();

        // 3. Pre-fetch role + joined_at per universe from `universe_members`.
        //    SQLite doesn't have a clean tuple-IN, so fetch the full set for the user.
        let mut role_by_universe: std::collections::HashMap<String, (String, Option<String>)> =
            std::collections::HashMap::new();
        if let Ok(mut stmt) = self.conn.prepare(
            "SELECT universe_key, role, joined_at FROM universe_members WHERE user_id = ?1",
        ) {
            for row in stmt
                .query_map(params![user_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1).unwrap_or_else(|_| "viewer".into()),
                        row.get::<_, Option<String>>(2).ok().flatten(),
                    ))
                })
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
            {
                role_by_universe.insert(row.0, (row.1, row.2));
            }
        }

        // 4. Pre-fetch subscription state per universe.
        let mut subscriber_universes: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        if let Ok(mut stmt) = self
            .conn
            .prepare("SELECT universe_key FROM subscriptions WHERE user_id = ?1")
        {
            for key in stmt
                .query_map(params![user_id], |row| row.get::<_, String>(0))
                .into_iter()
                .flatten()
                .filter_map(|r| r.ok())
            {
                subscriber_universes.insert(key);
            }
        }

        // 5. Build entries.
        universes
            .into_iter()
            .map(|u| {
                let is_owner = u.owner_id == user_id;
                let member = role_by_universe.get(&u.key);
                let is_member = member.is_some();
                let is_subscriber = subscriber_universes.contains(&u.key);

                let role = if is_owner {
                    "owner".to_string()
                } else if let Some((r, _)) = member {
                    r.clone()
                } else if is_subscriber {
                    "subscriber".to_string()
                } else {
                    "viewer".to_string()
                };

                let mut metadata = serde_json::Map::new();
                if let Some((_, joined_at)) = member
                    && let Some(joined) = joined_at
                {
                    metadata.insert(
                        "joined_at".into(),
                        serde_json::Value::String(joined.clone()),
                    );
                }
                // Quilombo metadata only attaches to the quilomboaraucaria universe.
                if u.key == "quilomboaraucaria"
                    && let Some(meta) = quilombo_meta.as_ref()
                    && let Some(obj) = meta.as_object()
                {
                    for (k, v) in obj {
                        metadata.insert(k.clone(), v.clone());
                    }
                }

                crate::models::UserUniverseEntry {
                    key: u.key,
                    name: u.name,
                    role,
                    is_owner,
                    is_member,
                    is_subscriber,
                    metadata: serde_json::Value::Object(metadata),
                }
            })
            .collect()
    }

    // --- CO-191: Bucketed universe helpers ---

    fn row_to_universe_with_role(
        &self,
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<(crate::models::Universe, Option<String>)> {
        Ok((
            crate::models::Universe {
                key: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                owner_id: row.get(3)?,
                created_at: super::schema::parse_datetime(&row.get::<_, String>(4)?),
                is_template: row.get::<_, i64>(5)? != 0,
                is_public: row.get::<_, i64>(6)? != 0,
                content_count: row.get::<_, i64>(7).unwrap_or(0),
                requires_login: row.get::<_, i64>(8).unwrap_or(0) != 0,
                visibility: row.get::<_, String>(9).unwrap_or_else(|_| "private".into()),
                parent_key: None,
                anon_published_only: false,
                source_kind: None,
                source_url: None,
                source_last_event_at: None,
                source_mode: None,
                surface_dns: None,
            },
            row.get::<_, Option<String>>(10).unwrap_or(None),
        ))
    }

    fn attach_parent_key(
        &self,
        universes: Vec<(crate::models::Universe, Option<String>)>,
    ) -> Vec<crate::models::UniverseWithRole> {
        universes
            .into_iter()
            .map(|(mut u, role)| {
                u.parent_key = self
                    .conn
                    .query_row(
                        "SELECT parent_key FROM universes WHERE key = ?1",
                        params![u.key],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten();
                crate::models::UniverseWithRole { universe: u, role }
            })
            .collect()
    }

    // CO-191 list methods — all four must be panic-free under the storage
    // mutex (see feedback_no_panic_under_mutex memory + 2026-05-12 incident).
    // On any SQLite error, log + return empty so /me/universes degrades to a
    // 200 with empty buckets rather than poisoning the lock site-wide.
    pub fn list_owned_universes(&self, user_id: &str) -> Vec<crate::models::UniverseWithRole> {
        let mut stmt = match self.conn.prepare(
            "SELECT u.key, u.name, u.description, u.owner_id, u.created_at, \
             u.is_template, u.is_public, u.content_count, \
             COALESCE(u.requires_login, 0), COALESCE(u.visibility, 'private'), \
             'owner' AS role \
             FROM universes u \
             WHERE u.owner_id = ?1 AND COALESCE(u.hidden, 0) = 0 \
               AND u.deleted_at IS NULL AND u.archived_at IS NULL \
               AND u.is_template = 0 \
             ORDER BY u.name ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("list_owned_universes prepare: {e}");
                return Vec::new();
            }
        };
        let rows: Vec<_> =
            match stmt.query_map(params![user_id], |row| self.row_to_universe_with_role(row)) {
                Ok(r) => r.filter_map(|x| x.ok()).collect(),
                Err(e) => {
                    tracing::error!("list_owned_universes query: {e}");
                    return Vec::new();
                }
            };
        self.attach_parent_key(rows)
    }

    pub fn list_member_universes(&self, user_id: &str) -> Vec<crate::models::UniverseWithRole> {
        let mut stmt = match self.conn.prepare(
            "SELECT u.key, u.name, u.description, u.owner_id, u.created_at, \
             u.is_template, u.is_public, u.content_count, \
             COALESCE(u.requires_login, 0), COALESCE(u.visibility, 'private'), \
             um.role \
             FROM universe_members um \
             JOIN universes u ON u.key = um.universe_key \
             WHERE um.user_id = ?1 AND um.role != 'owner' \
               AND COALESCE(u.hidden, 0) = 0 \
               AND u.deleted_at IS NULL AND u.archived_at IS NULL \
               AND u.is_template = 0 \
             ORDER BY u.name ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("list_member_universes prepare: {e}");
                return Vec::new();
            }
        };
        let rows: Vec<_> =
            match stmt.query_map(params![user_id], |row| self.row_to_universe_with_role(row)) {
                Ok(r) => r.filter_map(|x| x.ok()).collect(),
                Err(e) => {
                    tracing::error!("list_member_universes query: {e}");
                    return Vec::new();
                }
            };
        self.attach_parent_key(rows)
    }

    pub fn list_subscribed_universes(&self, user_id: &str) -> Vec<crate::models::UniverseWithRole> {
        let mut stmt = match self.conn.prepare(
            "SELECT u.key, u.name, u.description, u.owner_id, u.created_at, \
             u.is_template, u.is_public, u.content_count, \
             COALESCE(u.requires_login, 0), COALESCE(u.visibility, 'private'), \
             'subscriber' AS role \
             FROM subscriptions s \
             JOIN universes u ON u.key = s.universe_key \
             WHERE s.user_id = ?1 \
               AND u.owner_id != ?1 \
               AND u.key NOT IN (SELECT universe_key FROM universe_members WHERE user_id = ?1) \
               AND COALESCE(u.hidden, 0) = 0 \
               AND u.deleted_at IS NULL AND u.archived_at IS NULL \
               AND u.is_template = 0 \
             ORDER BY u.name ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("list_subscribed_universes prepare: {e}");
                return Vec::new();
            }
        };
        // CO-314: the query references `?1` in three places but rusqlite
        // counts distinct placeholders, not occurrences. Passing three
        // params for one slot errored as "Wrong number of parameters
        // passed to query. Got 2, needed 1" and dropped every row —
        // every subscribe attempt looked like it failed in the SPA even
        // though the DB write succeeded. Pass exactly one; SQLite reuses
        // it for every `?1` reference.
        let rows: Vec<_> =
            match stmt.query_map(params![user_id], |row| self.row_to_universe_with_role(row)) {
                Ok(r) => r.filter_map(|x| x.ok()).collect(),
                Err(e) => {
                    tracing::error!("list_subscribed_universes query: {e}");
                    return Vec::new();
                }
            };
        self.attach_parent_key(rows)
    }

    pub fn list_discoverable_universes(
        &self,
        excluded_keys: &std::collections::HashSet<String>,
        limit: usize,
    ) -> Vec<crate::models::UniverseWithRole> {
        let mut stmt = match self.conn.prepare(
            // CO-319: only public-subscribable universes should appear in the
            // discoverable bucket. The previous `OR is_public = 1` clause let
            // through `public-static` universes (the timeline trio: tempo,
            // humanity, universo) which a user CANNOT subscribe to — clicking
            // their Subscribe button returned 400 from the storage layer.
            // Static universes remain reachable via direct URL (/tempo etc.);
            // they just don't show up in this subscribe-context list.
            "SELECT u.key, u.name, u.description, u.owner_id, u.created_at, \
             u.is_template, u.is_public, u.content_count, \
             COALESCE(u.requires_login, 0), COALESCE(u.visibility, 'private'), \
             NULL AS role \
             FROM universes u \
             WHERE u.visibility = 'public-subscribable' \
               AND u.is_template = 0 \
               AND COALESCE(u.hidden, 0) = 0 \
               AND u.deleted_at IS NULL AND u.archived_at IS NULL \
             ORDER BY u.content_count DESC, u.name ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("list_discoverable_universes prepare: {e}");
                return Vec::new();
            }
        };
        let rows: Vec<_> = match stmt.query_map([], |row| self.row_to_universe_with_role(row)) {
            Ok(r) => r
                .filter_map(|x| x.ok())
                .filter(|(u, _)| !excluded_keys.contains(&u.key))
                .take(limit)
                .collect(),
            Err(e) => {
                tracing::error!("list_discoverable_universes query: {e}");
                return Vec::new();
            }
        };
        self.attach_parent_key(rows)
    }

    // --- CO-96: soft-delete + archive lifecycle ---

    /// CO-96: send a universe to the trash (soft-delete). The row and all its
    /// data survive — restorable within the 30-day window via the trash view —
    /// but every listing query filters `deleted_at IS NULL`, so it vanishes
    /// from the sidebar, public listings, search and discovery.
    pub fn soft_delete_universe(&self, key: &str) -> rusqlite::Result<usize> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE universes SET deleted_at = ?1 WHERE key = ?2 AND deleted_at IS NULL",
            params![now, key],
        )
    }

    /// CO-96: archive a universe (soft-hide). Like delete it disappears from
    /// every listing, but it's framed as a reversible "put away" rather than a
    /// trash action. Cleared by [`Storage::restore_universe`].
    pub fn archive_universe(&self, key: &str) -> rusqlite::Result<usize> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE universes SET archived_at = ?1 WHERE key = ?2 AND archived_at IS NULL",
            params![now, key],
        )
    }

    /// CO-96: restore a trashed or archived universe — clears both timestamps so
    /// it reappears in the sidebar. Returns the number of rows affected (0 if the
    /// universe wasn't actually deleted/archived).
    pub fn restore_universe(&self, key: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE universes SET deleted_at = NULL, archived_at = NULL WHERE key = ?1",
            params![key],
        )
    }

    /// CO-96: list the caller's trashed + archived universes for the recovery
    /// ("trash") view. Returns `(key, name, description, deleted_at, archived_at)`
    /// for every universe the user owns that is currently soft-deleted or
    /// archived, newest action first.
    #[allow(clippy::type_complexity)]
    pub fn list_trashed_universes_for_owner(
        &self,
        user_id: &str,
    ) -> Vec<(String, String, String, Option<String>, Option<String>)> {
        let mut stmt = match self.conn.prepare(
            "SELECT key, name, description, deleted_at, archived_at \
             FROM universes \
             WHERE owner_id = ?1 \
               AND (deleted_at IS NOT NULL OR archived_at IS NOT NULL) \
             ORDER BY COALESCE(deleted_at, archived_at) DESC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("list_trashed_universes_for_owner prepare: {e}");
                return Vec::new();
            }
        };
        match stmt.query_map(params![user_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2).unwrap_or_default(),
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(e) => {
                tracing::error!("list_trashed_universes_for_owner query: {e}");
                Vec::new()
            }
        }
    }

    // --- Universe Members ---

    pub fn is_universe_member(&self, universe_key: &str, user_id: &str) -> bool {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
                params![universe_key, user_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count > 0
    }

    pub fn list_universe_members(&self, universe_key: &str) -> Vec<crate::models::UniverseMember> {
        let mut stmt = self.conn.prepare(
            "SELECT um.universe_key, um.user_id, um.role, um.joined_at, u.email, u.display_name \
             FROM universe_members um \
             LEFT JOIN users u ON um.user_id = u.id \
             WHERE um.universe_key = ?1 \
             ORDER BY um.joined_at ASC",
        ).expect("Failed to prepare list_universe_members");
        stmt.query_map(params![universe_key], |row| {
            Ok(crate::models::UniverseMember {
                universe_key: row.get(0)?,
                user_id: row.get(1)?,
                role: row.get(2)?,
                joined_at: parse_datetime(&row.get::<_, String>(3)?),
                email: row.get(4)?,
                display_name: row.get(5)?,
            })
        })
        .expect("Failed to list universe members")
        .filter_map(|r| r.ok())
        .collect()
    }

    pub fn add_universe_member(
        &mut self,
        universe_key: &str,
        user_id: &str,
        role: &str,
    ) -> anyhow::Result<crate::models::UniverseMember> {
        if self.get_universe(universe_key).is_none() {
            anyhow::bail!("Universe '{}' not found", universe_key);
        }
        // Note: user_id may refer to a quilombo user (not in the users table) — no FK check.
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) VALUES (?1, ?2, ?3, ?4)",
            params![universe_key, user_id, role, now_str],
        )?;
        let member = self.conn.query_row(
            "SELECT um.universe_key, um.user_id, um.role, um.joined_at, u.email, u.display_name \
             FROM universe_members um LEFT JOIN users u ON um.user_id = u.id \
             WHERE um.universe_key = ?1 AND um.user_id = ?2",
            params![universe_key, user_id],
            |row| {
                Ok(crate::models::UniverseMember {
                    universe_key: row.get(0)?,
                    user_id: row.get(1)?,
                    role: row.get(2)?,
                    joined_at: parse_datetime(&row.get::<_, String>(3)?),
                    email: row.get(4)?,
                    display_name: row.get(5)?,
                })
            },
        )?;
        Ok(member)
    }

    pub fn remove_universe_member(
        &mut self,
        universe_key: &str,
        user_id: &str,
    ) -> anyhow::Result<()> {
        // Prevent removing the owner
        let universe = self
            .get_universe(universe_key)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", universe_key))?;
        if universe.owner_id == user_id {
            anyhow::bail!("Cannot remove the owner from a universe");
        }
        self.conn.execute(
            "DELETE FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
            params![universe_key, user_id],
        )?;
        Ok(())
    }

    // --- Universe member role helper ---

    /// Return the role of a user in a universe, or None if not a member.
    pub fn universe_member_role(&self, universe_key: &str, user_id: &str) -> Option<String> {
        self.conn
            .query_row(
                "SELECT role FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
                params![universe_key, user_id],
                |row| row.get(0),
            )
            .ok()
    }

    // --- CO-49: Subscriptions ---

    /// 1.46.0: auto-subscribe a user to every universe flagged
    /// `default_for_new_users=1`. Idempotent. Called on signup and on every
    /// boot for existing users (one-time backfill — the `INSERT OR IGNORE`
    /// makes repeat calls cheap).
    pub fn subscribe_user_to_default_universes(&self, user_id: &str) -> anyhow::Result<usize> {
        let now_str = Utc::now().to_rfc3339();
        let mut stmt = self
            .conn
            .prepare("SELECT key FROM universes WHERE default_for_new_users = 1")?;
        let keys: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();
        let mut added = 0usize;
        for key in &keys {
            let n = self.conn.execute(
                "INSERT OR IGNORE INTO subscriptions (user_id, universe_key, subscribed_at) \
                 VALUES (?1, ?2, ?3)",
                params![user_id, key, now_str],
            )?;
            added += n;
        }
        Ok(added)
    }

    /// 1.46.0: subscribe every existing user to every default universe.
    /// Run once at boot so the v29 migration that flagged yggdrasil
    /// reaches users who already exist.
    pub fn backfill_default_subscriptions(&self) -> anyhow::Result<usize> {
        let mut stmt = self.conn.prepare("SELECT id FROM users")?;
        let user_ids: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();
        let mut added = 0usize;
        for uid in &user_ids {
            added += self.subscribe_user_to_default_universes(uid)?;
        }
        Ok(added)
    }

    /// Subscribe a user to a public-subscribable universe.
    pub fn subscribe_universe(&mut self, user_id: &str, universe_key: &str) -> anyhow::Result<()> {
        let universe = self
            .get_universe(universe_key)
            .ok_or_else(|| anyhow::anyhow!("Universe '{}' not found", universe_key))?;
        if universe.visibility != "public-subscribable" {
            anyhow::bail!("Universe '{}' is not public-subscribable", universe_key);
        }
        let now_str = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO subscriptions (user_id, universe_key, subscribed_at) \
             VALUES (?1, ?2, ?3)",
            params![user_id, universe_key, now_str],
        )?;
        Ok(())
    }

    /// Unsubscribe a user from a universe.
    pub fn unsubscribe_universe(
        &mut self,
        user_id: &str,
        universe_key: &str,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM subscriptions WHERE user_id = ?1 AND universe_key = ?2",
            params![user_id, universe_key],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------
    // Phase 8 step 1: content-addressed blob storage (1.70.0)
    // -------------------------------------------------------------------

    /// Store the bytes (typically an entry body), keyed by sha256.
    /// Returns the hex-encoded sha256 hash. Idempotent — re-storing the
    /// same bytes is a no-op (INSERT OR IGNORE).
    pub fn put_blob(&self, bytes: &[u8]) -> anyhow::Result<String> {
        use sha2::{Digest, Sha256};
        let hash = format!("{:x}", Sha256::digest(bytes));
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR IGNORE INTO blobs (hash, bytes, size, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![hash, bytes, bytes.len() as i64, now],
        )?;
        Ok(hash)
    }

    /// Fetch a blob's bytes by hash. Returns `None` if the hash isn't stored.
    pub fn get_blob(&self, hash: &str) -> Option<Vec<u8>> {
        self.conn
            .query_row(
                "SELECT bytes FROM blobs WHERE hash = ?1",
                params![hash],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .ok()
    }

    /// True if the blob exists. Cheaper than `get_blob` (no body read).
    pub fn has_blob(&self, hash: &str) -> bool {
        self.conn
            .query_row("SELECT 1 FROM blobs WHERE hash = ?1", params![hash], |_| {
                Ok(true)
            })
            .unwrap_or(false)
    }

    /// 1.73.0 (Phase 8 step 3): one-time backfill — walk every entry in
    /// every per-universe DB and put its body into the CAS blob store.
    /// Idempotent (INSERT OR IGNORE inside `put_blob`). Returns the
    /// (universes_scanned, entries_processed, blobs_added) tuple. Boot-
    /// time call is cheap on subsequent runs because the only work for
    /// already-stored bodies is a hash compute + an INSERT OR IGNORE
    /// that no-ops at the unique-key level.
    pub fn backfill_blobs_from_entries(&self) -> (usize, usize, usize) {
        let mut stmt = match self.conn.prepare("SELECT key FROM universes") {
            Ok(s) => s,
            Err(_) => return (0, 0, 0),
        };
        let keys: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        let mut universes_scanned = 0usize;
        let mut entries_processed = 0usize;
        let mut blobs_added = 0usize;

        for key in &keys {
            universes_scanned += 1;
            let uc = self.universe_pool.get_or_open(key);
            let guard = match uc.lock() {
                Ok(g) => g,
                Err(_) => continue,
            };
            let mut s = match guard.prepare("SELECT body FROM entries WHERE universe_key = ?1") {
                Ok(s) => s,
                Err(_) => continue,
            };
            let bodies: Vec<String> = match s.query_map(params![key], |row| row.get::<_, String>(0))
            {
                Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
                Err(_) => continue,
            };
            drop(s);
            drop(guard);

            for body in &bodies {
                entries_processed += 1;
                use sha2::{Digest, Sha256};
                let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
                if !self.has_blob(&hash) && self.put_blob(body.as_bytes()).is_ok() {
                    blobs_added += 1;
                }
            }
        }

        tracing::info!(
            "blob backfill: scanned {universes_scanned} universe(s), processed {entries_processed} entries, {blobs_added} new blob(s) added"
        );
        (universes_scanned, entries_processed, blobs_added)
    }

    /// CO-322: Upsert a universe row for `co launch`.
    ///
    /// Idempotent via INSERT OR IGNORE — re-running in the same directory never
    /// creates a duplicate. If `public` is true the visibility is promoted to
    /// `public-subscribable` even when the row already existed (so you can run
    /// `co launch --public` on an existing private universe to make it public).
    ///
    /// Returns `true` when the row was newly inserted, `false` when it already
    /// existed.
    pub fn ensure_local_universe(&mut self, key: &str, name: &str, public: bool) -> bool {
        let now_str = chrono::Utc::now().to_rfc3339();
        let visibility = if public {
            "public-subscribable"
        } else {
            "private"
        };
        let is_public: i64 = if public { 1 } else { 0 };

        let inserted = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO universes \
                 (key, name, description, owner_id, created_at, is_template, is_public, visibility) \
                 VALUES (?1, ?2, '', 'system', ?3, 0, ?4, ?5)",
                rusqlite::params![key, name, now_str, is_public, visibility],
            )
            .unwrap_or(0);

        if public && inserted == 0 {
            let _ = self.conn.execute(
                "UPDATE universes SET visibility = 'public-subscribable', is_public = 1 \
                 WHERE key = ?1",
                rusqlite::params![key],
            );
        }

        inserted > 0
    }

    /// CO-322: Count entries in a universe by type for `co launch` summary output.
    ///
    /// Returns `(pages, tasks, projects)` where `pages` is all entries whose
    /// `entry_type` is not `task` or `project`.
    pub fn count_universe_entries(&self, universe_key: &str) -> (i64, i64, i64) {
        let uc = self.universe_pool.get_or_open(universe_key);

        let query_count = |sql: &str| -> i64 {
            uc.lock()
                .ok()
                .and_then(|g| g.query_row(sql, [], |r| r.get::<_, i64>(0)).ok())
                .unwrap_or(0)
        };

        let pages =
            query_count("SELECT COUNT(*) FROM entries WHERE entry_type NOT IN ('task', 'project')");
        let tasks = query_count("SELECT COUNT(*) FROM entries WHERE entry_type = 'task'");
        let projects = query_count("SELECT COUNT(*) FROM entries WHERE entry_type = 'project'");

        (pages, tasks, projects)
    }
}

#[cfg(test)]
mod tests {
    use super::super::Storage;

    fn make_storage() -> (Storage, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path().to_str().unwrap());
        (storage, dir)
    }

    /// CO-98: setting `parent_key` on a universe and reading it back via
    /// `get_universe` preserves the value (round-trip test).
    #[test]
    fn test_parent_key_round_trip() {
        let (storage, _dir) = make_storage();

        // Create two universes via direct SQL (Storage has no public create API
        // without an owner, but we can seed via raw connection).
        let now = chrono::Utc::now().to_rfc3339();
        storage
            .conn()
            .execute(
                "INSERT INTO universes \
                 (key, name, description, owner_id, created_at, is_template, is_public, visibility) \
                 VALUES ('parent-u', 'Parent', '', 'usr1', ?1, 0, 1, 'public-static'), \
                        ('child-u',  'Child',  '', 'usr1', ?1, 0, 1, 'public-static')",
                rusqlite::params![now],
            )
            .unwrap();

        // Set parent_key on child.
        storage
            .conn()
            .execute(
                "UPDATE universes SET parent_key = 'parent-u' WHERE key = 'child-u'",
                [],
            )
            .unwrap();

        // Round-trip: get_universe should return parent_key.
        let child = storage.get_universe("child-u").expect("child must exist");
        assert_eq!(
            child.parent_key.as_deref(),
            Some("parent-u"),
            "get_universe must return the stored parent_key"
        );

        let parent = storage.get_universe("parent-u").expect("parent must exist");
        assert!(
            parent.parent_key.is_none(),
            "top-level universe must have no parent_key"
        );
    }

    /// CO-98: `list_children_of_universe` returns only direct children of the
    /// given parent and excludes deleted/archived universes.
    #[test]
    fn test_list_children_of_universe() {
        let (storage, _dir) = make_storage();
        let now = chrono::Utc::now().to_rfc3339();

        storage
            .conn()
            .execute(
                "INSERT INTO universes \
                 (key, name, description, owner_id, created_at, is_template, is_public, visibility, parent_key) \
                 VALUES \
                   ('hub',      'Hub',      '', 'usr1', ?1, 0, 1, 'public-static', NULL), \
                   ('child-a',  'Child A',  '', 'usr1', ?1, 0, 1, 'public-static', 'hub'), \
                   ('child-b',  'Child B',  '', 'usr1', ?1, 0, 1, 'public-static', 'hub'), \
                   ('orphan',   'Orphan',   '', 'usr1', ?1, 0, 1, 'public-static', NULL), \
                   ('deleted',  'Deleted',  '', 'usr1', ?1, 0, 1, 'public-static', 'hub')",
                rusqlite::params![now],
            )
            .unwrap();

        // Soft-delete the 'deleted' child so it should be excluded.
        storage
            .conn()
            .execute(
                "UPDATE universes SET deleted_at = ?1 WHERE key = 'deleted'",
                rusqlite::params![now],
            )
            .unwrap();

        let children = storage.list_children_of_universe("hub");
        assert_eq!(children.len(), 2, "exactly two live children expected");
        let keys: Vec<&str> = children.iter().map(|u| u.key.as_str()).collect();
        assert!(keys.contains(&"child-a"), "child-a must appear");
        assert!(keys.contains(&"child-b"), "child-b must appear");
        assert!(!keys.contains(&"deleted"), "deleted child must be excluded");

        // Top-level universe has no children.
        assert!(
            storage.list_children_of_universe("orphan").is_empty(),
            "top-level universe must return empty children list"
        );
    }
}
