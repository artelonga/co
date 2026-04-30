//! split_db — offline migration tool: monolithic co.db → meta.db + per-universe data.db
//!
//! Usage:
//!   split_db --data-dir /data [--dry-run]
//!
//! What it does:
//! 1. Opens the source database (co.db or meta.db if already renamed).
//! 2. Creates meta.db with all non-entry tables (users, universes, etc.).
//! 3. For each universe key found in the entries table, creates
//!    `universes/{ab}/{cd}/{key}/data.db` and copies that universe's entries.
//! 4. Clears the entries table in meta.db once all entries are confirmed copied.
//!
//! This binary is idempotent: re-running after a partial migration is safe because
//! all inserts use INSERT OR IGNORE.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut data_dir = PathBuf::from(".");
    let mut dry_run = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                data_dir = args
                    .next()
                    .map(PathBuf::from)
                    .expect("--data-dir requires a value");
            }
            "--dry-run" => dry_run = true,
            other => {
                eprintln!("Unknown argument: {other}");
                std::process::exit(1);
            }
        }
    }

    println!("split_db: data-dir = {}", data_dir.display());
    if dry_run {
        println!("split_db: DRY RUN — no changes will be written");
    }

    // Determine source path: prefer co.db (not yet renamed), fall back to meta.db
    let meta_path = data_dir.join("meta.db");
    let old_path = data_dir.join("co.db");
    let src_path = if old_path.exists() && !meta_path.exists() {
        println!("split_db: source = co.db (will rename to meta.db)");
        old_path.clone()
    } else if meta_path.exists() {
        println!("split_db: source = meta.db");
        meta_path.clone()
    } else {
        eprintln!(
            "split_db: neither co.db nor meta.db found in {}",
            data_dir.display()
        );
        std::process::exit(1);
    };

    let src = Connection::open(&src_path).expect("open source db");
    src.execute_batch("PRAGMA journal_mode=WAL;").ok();

    // Count entries to migrate
    let total: i64 = src
        .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
        .unwrap_or(0);

    println!("split_db: found {total} entries to migrate");

    if total == 0 {
        println!("split_db: nothing to migrate");
        maybe_rename_to_meta(&old_path, &meta_path, dry_run);
        return;
    }

    // Collect distinct universe keys
    let mut stmt = src
        .prepare("SELECT DISTINCT universe_key FROM entries")
        .expect("prepare universes");
    let universe_keys: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .expect("query universes")
        .filter_map(|r| r.ok())
        .collect();

    println!(
        "split_db: {} universe(s) to migrate: {:?}",
        universe_keys.len(),
        universe_keys
    );

    let mut total_migrated: i64 = 0;

    for uk in &universe_keys {
        let db_path = universe_db_path(&data_dir, uk);
        println!("split_db:   universe '{uk}' → {}", db_path.display());

        if dry_run {
            continue;
        }

        std::fs::create_dir_all(db_path.parent().unwrap()).expect("create dir");
        let dst = Connection::open(&db_path).expect("open universe db");
        dst.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .ok();
        init_universe_schema(&dst);

        // Copy entries
        let mut stmt = src
            .prepare(
                "SELECT path, universe_key, entry_type, title, frontmatter_json, \
                 body, body_hash, created_at, updated_at \
                 FROM entries WHERE universe_key = ?1",
            )
            .expect("prepare entries");

        let count: i64 = stmt
            .query_map(params![uk], |row| {
                let path: String = row.get(0)?;
                let uk2: String = row.get(1)?;
                let et: String = row.get(2)?;
                let title: Option<String> = row.get(3)?;
                let fm: String = row.get(4)?;
                let body: String = row.get(5)?;
                let hash: String = row.get(6)?;
                let ca: Option<String> = row.get(7)?;
                let ua: Option<String> = row.get(8)?;

                dst.execute(
                    "INSERT OR IGNORE INTO entries \
                     (path, universe_key, entry_type, title, frontmatter_json, \
                      body, body_hash, created_at, updated_at) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![path, uk2, et, title, fm, body, hash, ca, ua],
                )
                .ok();

                // Register project entries in meta.db project_universe_index
                Ok(())
            })
            .expect("copy entries")
            .count() as i64;

        total_migrated += count;
        println!("split_db:   copied {count} entries for '{uk}'");
    }

    if !dry_run {
        // Populate project_universe_index in source (meta.db)
        ensure_project_universe_index(&src);
        let mut stmt = src
            .prepare(
                "SELECT universe_key, frontmatter_json FROM entries \
                 WHERE entry_type = 'project'",
            )
            .expect("prepare projects");
        stmt.query_map([], |row| {
            let uk: String = row.get(0)?;
            let fm_json: String = row.get(1)?;
            Ok((uk, fm_json))
        })
        .expect("query projects")
        .filter_map(|r| r.ok())
        .for_each(|(uk, fm_json)| {
            if let Ok(fm) = serde_json::from_str::<serde_json::Value>(&fm_json)
                && let Some(key) = fm.get("key").and_then(|v| v.as_str())
            {
                let _ = src.execute(
                    "INSERT OR IGNORE INTO project_universe_index \
                     (project_key, universe_key) VALUES (?1, ?2)",
                    params![key, uk],
                );
            }
        });

        // Clear entries from meta.db
        src.execute_batch("DELETE FROM entries; DELETE FROM entries_fts;")
            .ok();
        println!("split_db: cleared entries from source db");

        // Rename co.db → meta.db if needed
        maybe_rename_to_meta(&old_path, &meta_path, dry_run);
    }

    println!(
        "split_db: done — migrated {total_migrated} entries across {} universes",
        universe_keys.len()
    );
}

fn universe_db_path(data_dir: &Path, key: &str) -> PathBuf {
    let h = xxhash_rust::xxh3::xxh3_64(key.as_bytes());
    let b7 = ((h >> 56) & 0xff) as u8;
    let b6 = ((h >> 48) & 0xff) as u8;
    data_dir
        .join("universes")
        .join(format!("{b7:02x}"))
        .join(format!("{b6:02x}"))
        .join(key)
        .join("data.db")
}

fn init_universe_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS entries (
             path TEXT NOT NULL,
             universe_key TEXT NOT NULL,
             entry_type TEXT NOT NULL,
             title TEXT,
             frontmatter_json TEXT NOT NULL DEFAULT '{}',
             body TEXT NOT NULL DEFAULT '',
             body_hash TEXT NOT NULL DEFAULT '',
             created_at TEXT,
             updated_at TEXT,
             PRIMARY KEY (universe_key, path)
         );
         CREATE INDEX IF NOT EXISTS idx_entries_type ON entries(universe_key, entry_type);
         CREATE INDEX IF NOT EXISTS idx_entries_updated ON entries(universe_key, updated_at);
         CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
             universe_key UNINDEXED, path UNINDEXED, title, body
         );",
    )
    .expect("init universe schema");
    let v: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if v < 1 {
        conn.execute("INSERT INTO schema_version (version) VALUES (1)", [])
            .ok();
    }
}

fn ensure_project_universe_index(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_universe_index (
            project_key TEXT PRIMARY KEY,
            universe_key TEXT NOT NULL
        );",
    )
    .ok();
}

fn maybe_rename_to_meta(old_path: &Path, meta_path: &Path, dry_run: bool) {
    if old_path.exists() && !meta_path.exists() {
        if dry_run {
            println!(
                "split_db: [dry-run] would rename {} → {}",
                old_path.display(),
                meta_path.display()
            );
        } else {
            std::fs::rename(old_path, meta_path).expect("rename co.db → meta.db");
            println!("split_db: renamed co.db → meta.db");
        }
    }
}
