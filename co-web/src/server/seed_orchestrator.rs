use crate::config::WebConfig;
use crate::storage::Storage;

/// Seed all universe/project content on startup (template, quilombo, yggdrasil,
/// CO public pages, timeline, cleanup passes). Idempotent.
pub fn run_startup_seeds(config: &WebConfig) {
    let mut storage = Storage::new(&config.data_dir);
    let had_template = storage.template_exists();
    // CO-279: undo the never-shipped CO-254 rename (CO → TUTORIAL) by
    // dropping any stale `projects/TUTORIAL/*` entries from the template
    // before `seed_template_universe` re-seeds `CO` on fresh installs.
    storage.migrate_template_project_rename();
    storage.seed_template_universe();
    storage.reseed_template_content_pages();
    // Force theme preset back to 'modern' so the public landing page
    // always renders with the intended default — old migrations could
    // have left it on 'scholarly-light'.
    storage.ensure_template_theme_preset("modern");
    if had_template {
        tracing::info!(
            "Template refreshed: project+tasks seeded if missing, content pages reseeded, theme pinned to 'modern'"
        );
    } else {
        tracing::info!("No template universe found — seeded fresh template");
    }
    // CO-41: seed quilomboaraucaria public universe once on first boot.
    if !storage.quilombo_universe_exists() {
        tracing::info!("Seeding quilomboaraucaria universe...");
        storage.seed_quilombo_universe();
        tracing::info!("quilomboaraucaria universe seeded (public, quilombo theme)");
    }
    // CO-38: seed Yggdrasil minigames hub once on first boot.
    if !storage.yggdrasil_universe_exists() {
        tracing::info!("Seeding Yggdrasil universe...");
        storage.seed_yggdrasil_universe();
    }
    // Always reseed yggdrasil content pages so updates land for
    // existing installs (mirrors reseed_template_content_pages).
    storage.reseed_yggdrasil_content_pages();
    storage.cleanup_template_moved_pages();
    // CO-142 Phase C: hard-delete deprecated co-dev / co-experience rows on every boot.
    storage.delete_deprecated_universes();
    // CO-170: soft-hide empty parent placeholders + pre-merge mbya from
    // sidebar listings. Idempotent.
    storage.hide_deprecated_universes();
    // CO-170 Phase B (1.87.0): drop empty cross-leaked project stubs.
    // Idempotent — re-running after cleanup no-ops.
    let n_dropped = storage.cleanup_empty_projects();
    if n_dropped > 0 {
        tracing::info!("CO-170: cleaned up {n_dropped} empty project stub(s)");
    }
    // CO-170 Phase B (1.88.0): surgical moves of misplaced project content.
    // Per user direction: AL → artelonga, QA → quilomboaraucaria, CO-platform
    // projects (API/CW/DS/PLT) consolidated under `co`, tutorial leaks dropped.
    // Idempotent: source matches no rows after first successful run.
    let n_moved = storage.migrate_co170_phase_b();
    if n_moved > 0 {
        tracing::info!("CO-170 phase B: moved/dropped {n_moved} entries total");
    }
    // CO-170 phase B follow-up: rebuild project_universe_index on EVERY
    // boot — cheap (<100 rows), and we need it to happen on boots where
    // migrate_co170_phase_b finds nothing to move (the post-cleanup
    // steady state). Without this, the index stays in whatever state
    // the prior deploy left it in.
    storage.rebuild_project_universe_index();
    // Seed admin-owned content universes (artelonga, rfq, co) so they
    // appear in the sidebar without manual creation after every deploy.
    storage.seed_admin_content_universes();
    // CO-305: reseed co::public/* AFTER seed_admin_content_universes creates
    // the `co` universe row. On clean boot, the `co` universe doesn't exist
    // when this ran earlier (before line 44), so reseed_co_public_pages()
    // returned early and left CHANGELOG.md / public/index.md / seguranca etc.
    // unseeded — causing /co/changelog and /co/public/ to 404 in CI.
    // 2.7.20: transparency content (seguranca, licensa, infra*, …)
    // moved from `template::content/*` to `co::public/*`. Reseed
    // unconditionally on every boot; one-time cleanup above removes the
    // stale template copies.
    storage.reseed_co_public_pages();
    // CO-279: backfill a default project for every non-template universe
    // that has zero projects. Closes the "no project found" dead-end on
    // existing universes (yuri's private universe, sister-deployables, etc.)
    // that pre-date the create_universe default-project hook or were
    // imported via the seed-prod-universes.sh bootstrap script.
    let n_seeded = storage.backfill_default_projects();
    if n_seeded > 0 {
        tracing::info!("CO-279: seeded default project in {n_seeded} empty universe(s)");
    }
    // CO-261: placeholder pages in sister-repo universes until Wave B/C
    // wires their work/<space>/ task sync.
    storage.reseed_sister_repo_stubs();
    // Timeline trio (`tempo`, `humanity`, `universo`). Always re-seed —
    // the JSON manifests in the binary are the source of truth, and
    // `upsert_entry_row` makes overwriting safe.
    storage.seed_all_timeline_universes();
    // CO-142 Phase D: remove stale quilombo variants that have no documented purpose.
    storage.delete_stale_quilombo_variants();
    // CO-142 Phase B: recompute content_count from each universe's entry DB.
    // Seed paths (reseed_template_content_pages, seed_timeline_universe, etc.)
    // call upsert_entry_row but not increment_universe_content_count; this
    // corrects the drift on every boot.
    storage.recompute_content_counts();
    // Filesystem cruft cleanup: deletes /data/universes/<key>/ dirs from
    // a NARROW allowlist of known-deprecated keys (CO-142 Phase C+D).
    // 2026-05-02 fix: previous version was generic "any orphan", which
    // wiped UniversePool's hash-prefix shard dirs. Recovery handled in
    // rebuild_entries_from_filesystem below.
    storage.prune_orphan_universe_dirs();
    // 2026-05-02 recovery: re-ingest entries from filesystem for system
    // universes whose per-universe data.db got wiped by the buggy prune.
    // Idempotent — skipped per-universe when entries table already has rows.
    storage.rebuild_entries_from_filesystem(&[
        "template",
        "tempo",
        "humanity",
        "universo",
        "quilomboaraucaria",
        "artelonga",
        "rfq",
        "co",
        "yuri",
        "dados",
    ]);
    // Re-run content_count recompute now that per-universe entries are
    // populated; otherwise content_count stays at the pre-rebuild zero.
    storage.recompute_content_counts();
}

/// CO-317: seed sister universes from local repos.
///
/// For local dev: ingest markdown from each universe's `local_repo_path` into
/// the matching universe so localhost shows the same content that would be on
/// the deployed sister site. Bindings are stored in the `universes` table
/// (CO-330) — no hardcoded list. New repos can be added via
/// `PATCH /api/v1/universes/<key>/source` without a deploy.
///
/// Idempotent: upserts are no-ops for unchanged files; new files appear on the
/// next `co serve` restart.
pub fn run_sister_repo_seeds(config: &WebConfig) {
    let mut storage = Storage::new(&config.data_dir);

    // Read universe→repo bindings from DB (CO-330).
    let mut rows: Vec<(String, String, Option<String>)> = {
        let conn = storage.conn();
        let mut stmt = match conn.prepare(
            "SELECT key, local_repo_path, content_subdirs FROM universes \
             WHERE local_repo_path IS NOT NULL AND COALESCE(hidden, 0) = 0",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("run_sister_repo_seeds: prepare failed: {e}");
                return;
            }
        };
        match stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    };

    // CO-330 fallback: when CO_LOCAL_REPOS_DIR is set in env, it acts as the
    // parent directory and we re-resolve every universe's path against it
    // using the basename of the configured `local_repo_path`. Preserves the
    // pre-CO-330 mapping convention (env_var = parent, mapping had relative
    // name) so CO-321's e2e test (which writes <tempdir>/ArteLonga/docs/page.md
    // and sets CO_LOCAL_REPOS_DIR=<tempdir>) keeps working without DB writes.
    //
    // Universes with NULL local_repo_path also get a chance: try
    // <env>/<universe_key>.
    if let Ok(env_dir) = std::env::var("CO_LOCAL_REPOS_DIR") {
        let env_root = std::path::PathBuf::from(&env_dir);
        if env_root.exists() {
            // Rewrite paths of universes already in `rows` to point at <env>/<basename>.
            for (_k, path_str, _subdirs) in rows.iter_mut() {
                if let Some(basename) = std::path::Path::new(&*path_str)
                    .file_name()
                    .and_then(|n| n.to_str())
                {
                    let candidate = env_root.join(basename);
                    if candidate.exists() {
                        *path_str = candidate.to_string_lossy().to_string();
                    }
                }
            }
            // Also pick up universes with NULL local_repo_path when <env>/<key> exists.
            let covered: std::collections::HashSet<String> =
                rows.iter().map(|(k, _, _)| k.clone()).collect();
            let conn = storage.conn();
            if let Ok(mut stmt) = conn.prepare(
                "SELECT key FROM universes WHERE local_repo_path IS NULL AND COALESCE(hidden, 0) = 0",
            ) && let Ok(extra) = stmt.query_map([], |r| r.get::<_, String>(0))
            {
                for k in extra.flatten() {
                    if covered.contains(&k) {
                        continue;
                    }
                    let candidate = env_root.join(&k);
                    if candidate.exists() {
                        rows.push((k, candidate.to_string_lossy().to_string(), None));
                    }
                }
            }
        }
    }

    for (universe_key, repo_path_str, subdirs_json) in &rows {
        // Expand `~/` to the user home directory.
        let repo_path = if let Some(rel) = repo_path_str.strip_prefix("~/") {
            dirs::home_dir()
                .map(|h| h.join(rel))
                .unwrap_or_else(|| std::path::PathBuf::from(repo_path_str))
        } else {
            std::path::PathBuf::from(repo_path_str)
        };

        if !repo_path.exists() {
            continue;
        }

        // Parse content_subdirs; default to ["docs", "content"] on NULL/invalid.
        let subdirs: Vec<String> = subdirs_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .filter(|v: &Vec<String>| !v.is_empty())
            .unwrap_or_else(|| vec!["docs".into(), "content".into()]);
        let subdirs_refs: Vec<&str> = subdirs.iter().map(|s| s.as_str()).collect();

        // CO-319: pass 0 (never skip) so edits to local repo files are picked
        // up on the next `co serve` restart.
        storage.seed_universe_from_local_repo(universe_key, &repo_path, &subdirs_refs, 0);
        let work_dir = repo_path.join("work");
        if work_dir.exists() {
            storage.seed_universe_work_tasks_from_local(universe_key, &work_dir, 0);
        }
    }
}

/// CO-142 Phase E: refresh data/co/ from bundled /app/seed-co/ on every boot.
/// The seed dir is injected at Docker build time (COPY work/co/ /app/seed-co/).
/// This keeps the dev board in sync with repo state without writing back.
pub fn run_co142_refresh(config: &WebConfig) {
    // CO-310: use resolve_seed_co_dir() so this works on Fly (/app/seed-co),
    // local dev (walks up to work/co/), or via CO_SEED_CO_DIR env override.
    let co_dir = std::path::Path::new(&config.data_dir).join("co");
    let Some(seed_src) = super::uat_boot::resolve_seed_co_dir() else {
        return;
    };
    match super::uat_boot::copy_dir_all(&seed_src, &co_dir) {
        Ok(()) => tracing::info!("CO-142: refreshed data/co/ from {}", seed_src.display()),
        Err(e) => tracing::warn!("CO-142: could not refresh data/co/: {e}"),
    }
    // CO-142 follow-up (2026-05-02): Phase E populated /data/co/ for
    // the dev_board admin scan, but the SPA's /co/co board reads from
    // the per-universe `entries` table — which stayed empty, hence
    // user report "co has 0 entries, we have 140 tasks". This pass
    // bridges the gap by upserting each CO-*.md into the `co`
    // universe's entries at path tasks/<filename>.
    let mut storage = Storage::new(&config.data_dir);
    storage.seed_co_universe_tasks(&seed_src);
    // CO-264: also seed well-known root-level files (CHANGELOG.md,
    // README.md, LICENSE.md) from the parent of the seed dir.
    if let Some(root) = seed_src.parent() {
        storage.reseed_co_root_files(root);
    }
}

/// CO-85: seed admin user from env (idempotent, runs in any env).
/// CO-90 (preview): also ensure the seeded admin is a member of every
/// existing system universe so the SPA shows them post-login.
pub fn run_admin_seeding(config: &WebConfig) {
    let email = std::env::var("CO_SEED_ADMIN_EMAIL").ok();
    let hash = std::env::var("CO_SEED_ADMIN_PASSWORD_HASH").ok();
    if let (Some(email), Some(hash)) = (email, hash) {
        let mut storage = Storage::new(&config.data_dir);
        if let Err(e) = storage.seed_admin_user_from_env(&email, &hash) {
            tracing::error!("Failed to seed admin user from env: {e}");
        }
        if let Err(e) = storage.ensure_admin_universe_memberships(&email) {
            tracing::error!("Failed to ensure admin universe memberships: {e}");
        }
        // 1.46.0: subscribe every existing user to default universes
        // (currently just yggdrasil) so the v29 migration's flag
        // actually reaches their sidebar.
        match storage.backfill_default_subscriptions() {
            Ok(0) => {}
            Ok(n) => tracing::info!("Default-subscriptions backfill: added {n} row(s)"),
            Err(e) => tracing::error!("backfill_default_subscriptions: {e}"),
        }
        // CO-165 (1.85.0): every user with a `users.email` set gets that
        // email as a verified recovery channel, so `forgot-password`
        // Just Works without an extra add-channel step. Idempotent —
        // existing channels are re-confirmed verified, never duplicated.
        let n_channels = storage.backfill_email_recovery_channels();
        if n_channels > 0 {
            tracing::info!(
                "Recovery channel backfill: {n_channels} user(s) have a verified email channel"
            );
        }
        // CO-172 Phase 2: backfill recovery channels for linked quilombo users.
        let n_quilombo = storage.backfill_quilombo_recovery_channels();
        if n_quilombo > 0 {
            tracing::info!("Quilombo recovery channel backfill: {n_quilombo} user(s) promoted");
        }
        // CO-176 deployment-readiness: surface unbridged quilombo users so
        // operators can spot a regression. Counts `quilombo_usuarios`
        // rows where `linked_co_user_id IS NULL` — every quilombo signup
        // since 1.91.3 should be linked synchronously, so a non-zero
        // count here means either pre-1.91.3 legacy rows or a bridge
        // regression.
        let unbridged: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM quilombo_usuarios WHERE linked_co_user_id IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if unbridged > 0 {
            tracing::warn!(
                "CO-176 integrity: {unbridged} quilombo user(s) without linked_co_user_id — \
                 legacy rows recover lazily via /recover, but new signups must always link"
            );
        }
        // 1.73.0 (Phase 8 step 3): backfill CAS blobs from existing
        // entries on every boot. Cheap after the first run (hash
        // collisions hit INSERT OR IGNORE no-op) — first run on prod
        // imports ~5K bodies into blobs.
        let (us, ents, added) = storage.backfill_blobs_from_entries();
        if added > 0 || ents > 0 {
            tracing::info!(
                "blob backfill at boot: {us} universe(s), {ents} entries, {added} new blob(s)"
            );
        }
        // Re-home universes whose original owner user_id is no longer in
        // the users table (e.g. after a wipe / re-seed). Without this, the
        // universe remains in the DB but the new admin can't see it.
        match storage.rescue_orphan_universes(&email) {
            Ok(0) => {}
            Ok(n) => {
                tracing::info!("Re-homed {n} orphan universe(s) to {email}");
            }
            Err(e) => {
                tracing::error!("Failed to rescue orphan universes: {e}");
            }
        }
        // Force the well-known personal universes (bootstrapped via
        // scripts/seed-prod-universes.sh) to belong to the current admin
        // user, even when their prior owner_id is still a valid (stale)
        // user — not caught by rescue_orphan_universes.
        // 2026-05-02: free up legacy `*@co.local` username slugs so the
        // real admin can claim their email-prefix slug. Then derive the
        // admin's username from the email prefix.
        if let Err(e) = storage.free_legacy_co_local_usernames() {
            tracing::warn!("free_legacy_co_local_usernames failed: {e}");
        }
        if let Err(e) = storage.ensure_admin_username(&email) {
            tracing::warn!("ensure_admin_username failed: {e}");
        }
        // 2026-05-02: include `yuri` so the admin's slug-keyed personal
        // universe is owned by them (not the legacy yuri@co.local test
        // user that previously held it). Per user directive: "always use
        // slug as user name by default" — admin's slug is `yuri`, the
        // universe key matches.
        const PERSONAL_KEYS: &[&str] = &["artelonga", "rfq", "yuri"];
        match storage.ensure_admin_owns_personal_universes(&email, PERSONAL_KEYS) {
            Ok(0) => {}
            Ok(n) => {
                tracing::info!("Re-homed {n} personal universe(s) to {email}");
            }
            Err(e) => {
                tracing::error!("Failed to ensure admin owns personal universes: {e}");
            }
        }
        // Sweep "Meu Co" clutter from admin's sidebar — anon-clone
        // universes that an earlier rescue_orphan_universes grabbed.
        match storage.cleanup_admin_anon_clutter(&email) {
            Ok(0) => {}
            Ok(n) => {
                tracing::info!("Removed {n} stale anon-clone(s) from {email}");
            }
            Err(e) => {
                tracing::error!("Failed to clean admin anon clutter: {e}");
            }
        }
    }
}

/// CO-193: backfill default `general` chat room for pre-existing universes.
/// CO-198: backfill chat_room_members from universe_members.
/// CO-199: backfill default notification preferences.
/// CO-201: ensure push_subscriptions table exists.
pub fn backfill_chat_push(config: &WebConfig) {
    let chat_storage = Storage::new(&config.data_dir);
    let n = chat_storage.backfill_default_rooms();
    if n > 0 {
        tracing::info!("CO-193: seeded default general room for {n} universe(s)");
    }
    // CO-198: backfill chat_room_members from universe_members.
    let n_members = chat_storage.backfill_chat_room_members_from_universe_members();
    tracing::info!("CO-198: chat_room_members backfill: {n_members} row(s) total");
    // CO-199: backfill default notification_preferences for every existing user.
    let n_prefs = chat_storage.backfill_default_preferences();
    tracing::info!("CO-199: notification_preferences backfill: {n_prefs} row(s) inserted");
    // CO-209: rename the CO universe's default room to 'CO-geral'.
    let co_rename = chat_storage.conn().execute(
        "UPDATE chat_rooms SET name = 'CO-geral' \
         WHERE universe_key = 'co' AND slug = 'general' AND is_default = 1 AND name != 'CO-geral'",
        [],
    );
    if let Ok(n) = co_rename
        && n > 0
    {
        tracing::info!("CO-209: renamed CO universe general room to 'CO-geral'");
    }
    // CO-201: create push_subscriptions table if not yet present.
    chat_storage.ensure_push_subscriptions_table();
}
