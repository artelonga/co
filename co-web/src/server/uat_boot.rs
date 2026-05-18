use crate::config::WebConfig;
use crate::storage::Storage;

/// Recursively copy all files from `src` into `dst`, creating directories as needed.
pub(super) fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Runs all UAT-specific startup tasks when `CO_ENV=uat`.
///
/// # Reset flag
/// If `{data_dir}/uat-reset.flag` exists:
/// 1. Back up all users (with password hashes) from SQLite.
/// 2. Delete the SQLite database files.
/// 3. Remove anonymous universe directories.
/// 4. Re-open the database (runs all migrations from scratch).
/// 5. Restore the backed-up users.
/// 6. Re-seed the template universe.
/// 7. Delete the flag file.
///
/// # Always (after optional reset)
/// - Seed or update `yuri@uat.local` (tier=admin, password=`uat`).
/// - Clean up anonymous universes from the previous session.
/// - Seed `{data_dir}/co/` from `/app/seed-co/` if the directory is missing
///   (so the CO dev board has content on first boot).
///
/// Returns `true` if the reset flag was processed during this startup (CO-82
/// uses this to gate the prod-mirror task).
pub fn uat_startup(config: &WebConfig) -> bool {
    let data_dir = std::path::Path::new(&config.data_dir);
    let reset_flag = data_dir.join("uat-reset.flag");
    let reset_just_happened = reset_flag.exists();

    // --- Reset flag handling ---
    if reset_just_happened {
        tracing::info!("UAT: reset flag detected — resetting database...");

        // 0. Delete flag FIRST so a crash/restart doesn't re-trigger reset.
        let _ = std::fs::remove_file(&reset_flag);

        // 1. Back up users.
        let backup = {
            let storage = Storage::new(&config.data_dir);
            storage.get_all_users_with_hashes()
        };
        tracing::info!("UAT: backed up {} user(s)", backup.len());

        // 2. Delete SQLite database files.
        for suffix in &["co.db", "co.db-shm", "co.db-wal"] {
            let _ = std::fs::remove_file(data_dir.join(suffix));
        }

        // 3. Remove anonymous universe directories.
        let universes_dir = data_dir.join("universes");
        if universes_dir.exists()
            && let Ok(entries) = std::fs::read_dir(&universes_dir)
        {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("anon-") {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }

        // 4. Re-open DB (runs all migrations from scratch).
        let mut storage = Storage::new(&config.data_dir);

        // 5. Restore users.
        storage.restore_users_with_hashes(&backup);

        // 6. Re-seed all universes.
        if !storage.template_exists() {
            storage.seed_template_universe();
        }
        if !storage.quilombo_universe_exists() {
            storage.seed_quilombo_universe();
        }
        if !storage.yggdrasil_universe_exists() {
            storage.seed_yggdrasil_universe();
        }
        storage.reseed_yggdrasil_content_pages();

        drop(storage);

        tracing::info!("UAT: reset complete");
    }

    // --- Seed yuri@uat.local (idempotent) ---
    {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};

        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(b"uat", &salt)
            .expect("Argon2 hash failed")
            .to_string();

        let mut storage = Storage::new(&config.data_dir);
        if let Err(e) = storage.seed_uat_user(&hash) {
            tracing::error!("UAT: failed to seed yuri user: {e}");
        }

        // Add yuri as member of quilomboaraucaria so it appears in their sidebar
        let _ = storage.conn().execute(
            "INSERT OR IGNORE INTO universe_members (universe_key, user_id, role, joined_at) \
             VALUES ('quilomboaraucaria', 'usr_yuri_uat', 'admin', datetime('now'))",
            rusqlite::params![],
        );

        // --- Clean up anonymous universes from previous session ---
        let cleaned = storage.cleanup_anon_universes();
        if cleaned > 0 {
            tracing::info!("UAT: removed {cleaned} anonymous universe(s) from previous session");
        }

        // CO-45: snapshot current UAT state so mutations can be diffed later.
        match crate::uat_routes::create_snapshot(&config.data_dir, &storage) {
            Ok(snap) => tracing::info!("UAT: snapshot {} created", snap.version),
            Err(e) => tracing::warn!("UAT: could not create snapshot: {e}"),
        }
    }

    // --- Seed co-dev tasks from bundled data ---
    let co_dir = data_dir.join("co");
    if !co_dir.exists() {
        let seed_src = std::path::Path::new("/app/seed-co");
        if seed_src.exists() {
            match copy_dir_all(seed_src, &co_dir) {
                Ok(()) => tracing::info!("UAT: seeded co-dev tasks from /app/seed-co"),
                Err(e) => tracing::warn!("UAT: could not seed co-dev tasks: {e}"),
            }
        } else {
            tracing::warn!(
                "UAT: /app/seed-co not found — co-dev board will be empty. \
                 Add co task files manually at {}/co/",
                config.data_dir
            );
        }
    }

    reset_just_happened
}
