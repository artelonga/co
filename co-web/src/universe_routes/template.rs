use super::*;

// ---------------------------------------------------------------------------
// CO-72: Doc-generator job submission and status
// ---------------------------------------------------------------------------

/// Request body for `POST /api/v1/universes/:slug/jobs/doc-gen`.
#[derive(Debug, serde::Deserialize)]
pub struct DocGenRequest {
    /// One of: scaladoc, sphinx, mkdocs, redoc, rustdoc, jsdoc.
    pub format: String,
    /// Relative or absolute path to the source directory (e.g. `src/main/scala`).
    pub source_dir: String,
    /// Entry type tag for generated entries (e.g. `doc.scala`). Defaults to
    /// the adapter's built-in output type when empty.
    #[serde(default)]
    pub output_type: String,
}

/// POST /api/v1/universes/:slug/jobs/doc-gen — submit a doc-gen job (owner only).
pub async fn submit_doc_gen_job(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user_id: UserId,
    Json(body): Json<DocGenRequest>,
) -> Result<impl IntoResponse, AppError> {
    use std::str::FromStr as _;

    // Validate format early.
    let doc_format = crate::doc_gen::DocFormat::from_str(&body.format).map_err(|_| {
        AppError::BadRequest(format!(
            "Unknown doc format '{}'. Supported: scaladoc, sphinx, mkdocs, redoc, rustdoc, jsdoc",
            body.format
        ))
    })?;

    if body.source_dir.trim().is_empty() {
        return Err(AppError::BadRequest("source_dir cannot be empty".into()));
    }

    let storage = lock_storage(&state);
    let universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

    if universe.owner_id != user_id.0 {
        return Err(AppError::Forbidden(
            "Only the owner can submit doc-gen jobs".into(),
        ));
    }

    let output_type = if body.output_type.is_empty() {
        format!("doc.{}", doc_format.as_str())
    } else {
        body.output_type.clone()
    };

    let payload = crate::job_queue::DocGenPayload {
        format: body.format,
        source_dir: body.source_dir,
        output_type,
        limits: crate::doc_gen::ResourceLimits::default(),
    };

    let job_id = crate::job_queue::enqueue_doc_gen(storage.conn(), &slug, &payload)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "job_id": job_id })),
    ))
}

/// Last doc-gen error info returned by the status endpoint.
#[derive(Debug, serde::Serialize)]
pub struct DocGenErrorInfo {
    pub universe_key: String,
    pub error: Option<String>,
    pub error_at: Option<String>,
}

/// GET /api/v1/universes/:slug/jobs/doc-gen/last-error — last failure (owner only, auth inline).
pub async fn get_doc_gen_last_error(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> Result<Json<DocGenErrorInfo>, AppError> {
    let caller_id = extract_optional_user_id(&headers, &state)
        .ok_or_else(|| AppError::Unauthorized("Not authenticated".into()))?;

    let storage = lock_storage(&state);
    let universe = storage
        .get_universe(&slug)
        .ok_or_else(|| AppError::NotFound(format!("Universe '{}' not found", slug)))?;

    if universe.owner_id != caller_id {
        return Err(AppError::Forbidden(
            "Only the owner can view doc-gen errors".into(),
        ));
    }

    let (error, error_at): (Option<String>, Option<String>) = storage
        .conn()
        .query_row(
            "SELECT doc_gen_error, doc_gen_error_at FROM universes WHERE key = ?1",
            rusqlite::params![slug],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(DocGenErrorInfo {
        universe_key: slug,
        error,
        error_at,
    }))
}

/// Standalone router for the `/api/v1/themes` namespace (no auth layer).
pub fn themes_router() -> Router<AppState> {
    Router::new()
        .route("/available", get(get_available_themes))
        // Direct preset CSS — used by the SPA's user-level palette override so
        // we don't depend on any universe's stored theme_preset.
        // Route is `/{preset}` (without `.css`) because Axum's matchit doesn't
        // accept literal suffixes on dynamic segments. The handler still
        // tolerates a `.css` suffix on the preset name.
        .route("/{preset}", get(get_preset_theme_css))
}

// ---------------------------------------------------------------------------
// CO-162: Universe template scaffold + type audit
// ---------------------------------------------------------------------------

/// One entry in the type-check report.
#[derive(Debug, serde::Serialize)]
pub struct TypeError {
    pub path: String,
    pub issue: String,
}

/// Response for `POST /:slug/apply-template`.
#[derive(Debug, serde::Serialize)]
pub struct ApplyTemplateResponse {
    pub created: Vec<String>,
    pub skipped: Vec<String>,
    pub type_errors: Vec<TypeError>,
}

/// POST /api/v1/universes/:slug/apply-template
///
/// Creates standard scaffold files (CLAUDE.md, docs/api.md) when absent,
/// ensures the `doc` content type is registered in `_universe.yaml`, and
/// runs a type audit over all indexed entries. Idempotent — existing files
/// are skipped.
///
/// Auth: protected by `universe_writer_gate` middleware (owner or member).
pub async fn apply_template(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<axum::Json<ApplyTemplateResponse>, AppError> {
    use co::manifest::{ContentType, MANIFEST_FILENAME};
    use std::collections::HashSet;

    let universe_root = {
        let storage = lock_storage(&state);
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_root(&slug)
    };

    // --- 1. Load manifest ---
    let mut manifest_opt: Option<co::manifest::Manifest> = {
        std::fs::read(universe_root.join(MANIFEST_FILENAME))
            .ok()
            .and_then(|b| co::manifest::parse(&b).ok().map(|r| r.manifest))
    };

    // --- 2. Ensure `doc` content type is registered ---
    if let Some(ref mut m) = manifest_opt
        && !m.content_types.iter().any(|ct| ct.name == "doc")
    {
        m.content_types.push(ContentType {
            name: "doc".to_string(),
            schema: Default::default(),
            presentation: Default::default(),
            indexes: vec![],
        });
        if let Ok(yaml) = m.to_yaml() {
            let _ = std::fs::write(universe_root.join(MANIFEST_FILENAME), yaml.as_bytes());
        }
    }

    // Re-read (may have been updated above) to build the known-types set.
    let manifest = std::fs::read(universe_root.join(MANIFEST_FILENAME))
        .ok()
        .and_then(|b| co::manifest::parse(&b).ok().map(|r| r.manifest));

    let known_types: HashSet<String> = manifest
        .as_ref()
        .map(|m| m.content_types.iter().map(|ct| ct.name.clone()).collect())
        .unwrap_or_default();

    // --- 3. Gather universe metadata for template rendering ---
    let (universe_name, universe_desc) = {
        let storage = lock_storage(&state);
        let u = storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        (u.name.clone(), u.description.clone())
    };

    let type_names: Vec<String> = manifest
        .as_ref()
        .map(|m| m.content_types.iter().map(|ct| ct.name.clone()).collect())
        .unwrap_or_default();

    // --- 4. Create scaffold files ---
    let scaffold: Vec<(&str, String)> = vec![
        (
            "CLAUDE.md",
            build_claude_md(&universe_name, &universe_desc, &slug, &type_names),
        ),
        ("docs/api.md", build_api_md(&slug)),
    ];

    let mut created: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for (rel_path, body) in &scaffold {
        let disk_path = universe_root.join(rel_path);
        if disk_path.exists() {
            skipped.push(rel_path.to_string());
            continue;
        }
        let frontmatter = serde_json::json!({ "type": "doc", "title": rel_path });
        let entry = crate::entry_index::make_entry(rel_path, frontmatter, body);

        co::write_entry(&universe_root, &entry)
            .map_err(|e| AppError::Internal(format!("write {rel_path}: {e}")))?;

        {
            let uc = {
                let storage = lock_storage(&state);
                storage.universe_conn(&slug)
            };
            let guard = uc
                .lock()
                .map_err(|_| AppError::Internal("universe conn lock".into()))?;
            crate::entry_index::EntryIndex::new(&guard)
                .upsert(&slug, &entry)
                .map_err(|e| AppError::Internal(e.to_string()))?;
        }

        {
            let mut storage = lock_storage(&state);
            storage.increment_universe_content_count(&slug);
        }

        created.push(rel_path.to_string());
    }

    // --- 5. Type audit ---
    let type_errors = {
        let uc = {
            let storage = lock_storage(&state);
            storage.universe_conn(&slug)
        };
        let guard = uc
            .lock()
            .map_err(|_| AppError::Internal("universe conn lock".into()))?;
        run_type_check(&guard, &slug, &known_types)?
    };

    Ok(axum::Json(ApplyTemplateResponse {
        created,
        skipped,
        type_errors,
    }))
}

/// Scan all indexed entries and return those whose `entry_type` is missing
/// (`"unknown"`) or not declared in `_universe.yaml`.
fn run_type_check(
    conn: &rusqlite::Connection,
    universe_key: &str,
    known_types: &std::collections::HashSet<String>,
) -> Result<Vec<TypeError>, AppError> {
    let mut stmt = conn
        .prepare("SELECT path, entry_type FROM entries WHERE universe_key = ?1 ORDER BY path")
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let rows: Vec<(String, String)> = stmt
        .query_map(rusqlite::params![universe_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let mut errors = Vec::new();
    for (path, entry_type) in rows {
        if path == co::manifest::MANIFEST_FILENAME || path.starts_with('.') {
            continue;
        }
        if entry_type == "unknown" {
            errors.push(TypeError {
                path,
                issue: "missing type: field".into(),
            });
        } else if !known_types.is_empty() && !known_types.contains(&entry_type) {
            errors.push(TypeError {
                path,
                issue: format!("unknown type '{entry_type}' (not in _universe.yaml)"),
            });
        }
    }
    Ok(errors)
}

fn build_claude_md(name: &str, description: &str, slug: &str, types: &[String]) -> String {
    let ct_list = if types.is_empty() {
        "_(no content types declared — add them to `_universe.yaml`)_".to_string()
    } else {
        types
            .iter()
            .map(|t| format!("- `{t}`"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let desc_block = if description.trim().is_empty() {
        String::new()
    } else {
        format!("\n{}\n", description.trim())
    };
    format!(
        "# {name}\n{desc_block}\n\
         ## Universe\n\n\
         - **Slug**: `{slug}`\n\
         - **API base**: `/api/v1/universes/{slug}`\n\
         - **Viewer**: `/{slug}`\n\n\
         ## Content types\n\n{ct_list}\n\n\
         ## Working with this universe\n\n\
         All entries are `.md` files with YAML front-matter stored under the \
         universe data directory. The schema is declared in `_universe.yaml`.\n\n\
         Common queries:\n\n\
         ```bash\n\
         # List all entries\n\
         curl /api/v1/universes/{slug}/entries\n\n\
         # Filter by type\n\
         curl /api/v1/universes/{slug}/entries?type=<type>\n\n\
         # Full-text search\n\
         curl /api/v1/universes/{slug}/entries?q=<query>\n\n\
         # Universe schema\n\
         curl /api/v1/universes/{slug}/manifest\n\n\
         # Type audit\n\
         curl -X POST /api/v1/universes/{slug}/apply-template\n\
         ```\n\n\
         ## Conventions\n\n\
         - Every entry must include `type:` in its front-matter.\n\
         - Internal links use `[[path/to/entry]]` syntax (no `.md` extension).\n\
         - Asset references use `sha256:<hex>` in image/video `src` attributes.\n\
         - External URLs use standard markdown links `[label](https://...)`.\n"
    )
}

fn build_api_md(slug: &str) -> String {
    format!(
        "# CO API — {slug}\n\n\
         > Auto-generated scaffold. Edit to add universe-specific notes.\n\n\
         ## Base URL\n\n\
         ```\n/api/v1/universes/{slug}\n```\n\n\
         ## Auth\n\n\
         Include `Authorization: Bearer <token>`. Obtain via:\n\n\
         ```bash\n\
         # Request login code\n\
         POST /api/v1/auth/login  {{\"email\": \"you@example.com\"}}\n\n\
         # Exchange code for JWT\n\
         POST /api/v1/auth/verify  {{\"email\": \"...\", \"code\": \"...\"}}\n\
         ```\n\n\
         ## Endpoints\n\n\
         | Method | Path | Description |\n\
         |--------|------|-------------|\n\
         | GET | `/entries` | List entries (`?type=`, `?q=`, `?filter=`) |\n\
         | POST | `/entries` | Create entry |\n\
         | GET | `/entries/{{*path}}` | Read one entry |\n\
         | PUT | `/entries/{{*path}}` | Update entry |\n\
         | DELETE | `/entries/{{*path}}` | Delete entry |\n\
         | GET | `/entries/tags` | Tag counts |\n\
         | GET | `/entries/tree` | Hierarchical tree |\n\
         | GET | `/manifest` | Universe schema |\n\
         | POST | `/apply-template` | Re-run scaffold + type audit |\n\n\
         ## Schema\n\n\
         See [[_universe.yaml]] for declared content types.\n"
    )
}

// ---------------------------------------------------------------------------
// Bulk template + universe hub
// ---------------------------------------------------------------------------

/// Per-universe result inside `ApplyAllResponse`.
#[derive(Debug, serde::Serialize)]
pub struct UniverseTemplateResult {
    pub slug: String,
    pub name: String,
    pub content_count: i64,
    pub created: Vec<String>,
    pub skipped: Vec<String>,
    pub type_error_count: usize,
}

/// Response for `POST /apply-template-all`.
#[derive(Debug, serde::Serialize)]
pub struct ApplyAllResponse {
    pub results: Vec<UniverseTemplateResult>,
    pub hub_entry: Option<String>,
}

/// Request body for `POST /apply-template-all`.
#[derive(Debug, serde::Deserialize)]
pub struct ApplyAllRequest {
    /// Slug of the universe that should receive the auto-generated hub entry
    /// (e.g. your private `co` dev universe). Leave empty to skip hub creation.
    #[serde(default)]
    pub hub_universe: String,
}

/// POST /api/v1/universes/apply-template-all
///
/// Applies the standard scaffold (CLAUDE.md, docs/api.md) to every universe
/// the authenticated user owns, then writes a datos-style summary entry
/// (`universes.md`) into `hub_universe` (if supplied).
///
/// Auth: JWT required (owner scope per-universe — only owned universes touched).
pub async fn apply_template_all(
    State(state): State<AppState>,
    user_id: UserId,
    Json(body): Json<ApplyAllRequest>,
) -> Result<axum::Json<ApplyAllResponse>, AppError> {
    use co::manifest::MANIFEST_FILENAME;
    use std::collections::HashSet;

    // Collect universes owned by this user.
    let owned: Vec<crate::models::Universe> = {
        let storage = lock_storage(&state);
        storage
            .list_universes_for_user(&user_id.0)
            .into_iter()
            .filter(|u| u.owner_id == user_id.0)
            .collect()
    };

    let mut results: Vec<UniverseTemplateResult> = Vec::new();

    for universe in &owned {
        let slug = &universe.key.clone();
        let universe_root = {
            let storage = lock_storage(&state);
            storage.universe_root(slug)
        };

        // --- manifest: ensure doc type ---
        let mut manifest_opt: Option<co::manifest::Manifest> = {
            std::fs::read(universe_root.join(MANIFEST_FILENAME))
                .ok()
                .and_then(|b| co::manifest::parse(&b).ok().map(|r| r.manifest))
        };
        if let Some(ref mut m) = manifest_opt
            && !m.content_types.iter().any(|ct| ct.name == "doc")
        {
            m.content_types.push(co::manifest::ContentType {
                name: "doc".to_string(),
                schema: Default::default(),
                presentation: Default::default(),
                indexes: vec![],
            });
            if let Ok(yaml) = m.to_yaml() {
                let _ = std::fs::write(universe_root.join(MANIFEST_FILENAME), yaml.as_bytes());
            }
        }

        let manifest = std::fs::read(universe_root.join(MANIFEST_FILENAME))
            .ok()
            .and_then(|b| co::manifest::parse(&b).ok().map(|r| r.manifest));
        let known_types: HashSet<String> = manifest
            .as_ref()
            .map(|m| m.content_types.iter().map(|ct| ct.name.clone()).collect())
            .unwrap_or_default();
        let type_names: Vec<String> = manifest
            .as_ref()
            .map(|m| m.content_types.iter().map(|ct| ct.name.clone()).collect())
            .unwrap_or_default();

        // --- scaffold files ---
        let scaffold: Vec<(&str, String)> = vec![
            (
                "CLAUDE.md",
                build_claude_md(&universe.name, &universe.description, slug, &type_names),
            ),
            ("docs/api.md", build_api_md(slug)),
        ];
        let mut created: Vec<String> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for (rel, body_text) in &scaffold {
            let disk_path = universe_root.join(rel);
            if disk_path.exists() {
                skipped.push(rel.to_string());
                continue;
            }
            let fm = serde_json::json!({ "type": "doc", "title": rel });
            let entry = crate::entry_index::make_entry(rel, fm, body_text);
            if co::write_entry(&universe_root, &entry).is_ok() {
                let uc = {
                    let s = lock_storage(&state);
                    s.universe_conn(slug)
                };
                if let Ok(g) = uc.lock() {
                    let _ = crate::entry_index::EntryIndex::new(&g).upsert(slug, &entry);
                }
                {
                    let mut s = state.storage.lock();
                    s.increment_universe_content_count(slug);
                }
                created.push(rel.to_string());
            }
        }

        // --- type check ---
        let type_error_count = {
            let uc = {
                let s = lock_storage(&state);
                s.universe_conn(slug)
            };
            uc.lock()
                .ok()
                .and_then(|g| run_type_check(&g, slug, &known_types).ok())
                .map(|v| v.len())
                .unwrap_or(0)
        };

        results.push(UniverseTemplateResult {
            slug: slug.to_string(),
            name: universe.name.clone(),
            content_count: universe.content_count,
            created,
            skipped,
            type_error_count,
        });
    }

    // --- hub entry ---
    let hub_slug = body.hub_universe.trim().to_string();
    let hub_entry_path = if !hub_slug.is_empty() {
        let hub_root = {
            let storage = lock_storage(&state);
            if storage.get_universe(&hub_slug).is_none() {
                return Err(AppError::NotFound(format!(
                    "Hub universe '{hub_slug}' not found"
                )));
            }
            // Caller must own or be a member of the hub universe
            let is_ok = storage
                .conn()
                .query_row(
                    "SELECT 1 FROM universe_members WHERE universe_key = ?1 AND user_id = ?2",
                    rusqlite::params![&hub_slug, &user_id.0],
                    |_| Ok(true),
                )
                .unwrap_or(false)
                || storage
                    .get_universe(&hub_slug)
                    .is_some_and(|u| u.owner_id == user_id.0);
            if !is_ok {
                return Err(AppError::Forbidden(
                    "Not a member of the hub universe".into(),
                ));
            }
            storage.universe_root(&hub_slug)
        };

        let hub_body = build_hub_md(&results);
        let fm = serde_json::json!({
            "type": "doc",
            "title": "Universe Hub",
            "updated_at": chrono::Utc::now().to_rfc3339(),
        });
        let entry = crate::entry_index::make_entry("universes.md", fm, &hub_body);
        co::write_entry(&hub_root, &entry)
            .map_err(|e| AppError::Internal(format!("write hub: {e}")))?;
        let uc = {
            let s = lock_storage(&state);
            s.universe_conn(&hub_slug)
        };
        if let Ok(g) = uc.lock() {
            let _ = crate::entry_index::EntryIndex::new(&g).upsert(&hub_slug, &entry);
        }
        Some(format!("{hub_slug}/universes.md"))
    } else {
        None
    };

    Ok(axum::Json(ApplyAllResponse {
        results,
        hub_entry: hub_entry_path,
    }))
}

/// Generate the markdown body for the universe hub entry.
fn build_hub_md(results: &[UniverseTemplateResult]) -> String {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");
    let mut rows = String::new();
    for r in results {
        let template_ok = if r.created.is_empty() && !r.skipped.is_empty() {
            "✓"
        } else if !r.created.is_empty() {
            "✓ new"
        } else {
            "—"
        };
        let type_col = if r.type_error_count == 0 {
            "✓".to_string()
        } else {
            format!("⚠ {}", r.type_error_count)
        };
        rows.push_str(&format!(
            "| [[{slug}]] | {name} | {count} | {template} | {types} |\n",
            slug = r.slug,
            name = r.name,
            count = r.content_count,
            template = template_ok,
            types = type_col,
        ));
    }
    let total: i64 = results.iter().map(|r| r.content_count).sum();
    let total_errors: usize = results.iter().map(|r| r.type_error_count).sum();
    format!(
        "---\ntype: doc\ntitle: Universe Hub\n---\n\n\
         # Universe Hub\n\n\
         > Generated {now} — {n} universes, {total} entries total\n\n\
         | Universe | Name | Entries | Template | Types |\n\
         |----------|------|---------|----------|-------|\n\
         {rows}\n\
         **Total:** {total} entries across {n} universes — \
         {err} type {errlabel}.\n\n\
         To refresh: `POST /api/v1/universes/apply-template-all`\n",
        n = results.len(),
        err = total_errors,
        errlabel = if total_errors == 1 { "error" } else { "errors" },
    )
}

// ---------------------------------------------------------------------------
// Reindex
// ---------------------------------------------------------------------------

/// Response for `POST /:slug/reindex`.
#[derive(Debug, serde::Serialize)]
pub struct ReindexResponse {
    pub indexed: usize,
    pub errors: Vec<String>,
}

/// POST /api/v1/universes/:slug/reindex
///
/// Walk every `.md` file in the universe directory, parse frontmatter + body,
/// and upsert into the per-universe SQLite entry index. Idempotent — safe on
/// a live server. Also syncs `content_count` and invalidates query caches.
///
/// Auth: protected by `universe_writer_gate` middleware (owner or member).
pub async fn reindex(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<axum::Json<ReindexResponse>, AppError> {
    let universe_root = {
        let storage = lock_storage(&state);
        storage
            .get_universe(&slug)
            .ok_or_else(|| AppError::NotFound(format!("Universe '{slug}' not found")))?;
        storage.universe_root(&slug)
    };

    let disk_entries = co::scan_entries(&universe_root)
        .map_err(|e| AppError::Internal(format!("scan_entries: {e}")))?;

    let uc = {
        let storage = lock_storage(&state);
        storage.universe_conn(&slug)
    };
    let guard = uc
        .lock()
        .map_err(|_| AppError::Internal("universe conn lock".into()))?;
    let index = crate::entry_index::EntryIndex::new(&guard);

    let mut indexed = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for entry in &disk_entries {
        match index.upsert(&slug, entry) {
            Ok(()) => indexed += 1,
            Err(e) => errors.push(format!("{}: {e}", entry.path)),
        }
    }

    // Sync content_count to on-disk reality.
    {
        let storage = state.storage.lock();
        let _ = storage.conn().execute(
            "UPDATE universes SET content_count = ?1 WHERE key = ?2",
            rusqlite::params![disk_entries.len() as i64, &slug],
        );
    }

    state.cache.invalidate_universe(&slug);

    Ok(axum::Json(ReindexResponse { indexed, errors }))
}

/// Router for universe-level action endpoints that require the writer gate.
/// Merged into `universe_content_api` in `server::build_router`.
pub fn universe_actions_router() -> axum::Router<AppState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/{slug}/apply-template", post(apply_template))
        .route("/{slug}/reindex", post(reindex))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
