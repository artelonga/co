//! audit_serve — CO-439: allowlist-on-serve leak-surface audit.
//!
//! Usage:
//!   audit_serve [<data-dir>]        # defaults to $CO_DATA_DIR, then ./data
//!
//! For every universe, walks its served on-disk root (`<data-dir>/universes/<key>`)
//! for content files and cross-checks each against the per-universe published
//! index (the `entries` table). Files that exist on disk but are **absent from
//! the index** are reported as the "servable but not published" leak surface —
//! exactly what the surfaces allowlist (CO-439) now refuses to serve and what a
//! `.dockerignore` denylist could silently miss.
//!
//! Read-only: opens databases and reads the filesystem, never writes. Exit code
//! is 0 when no leak surface is found, 1 when at least one unindexed file
//! remains (so it can gate CI / a pre-deploy check), 2 on bad invocation.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use co_web::server::allowlist::unindexed_disk_paths;
use co_web::storage::Storage;

/// Files that are bookkeeping, not served content — never counted as leaks.
fn is_bookkeeping(rel: &str) -> bool {
    let leaf = rel.rsplit('/').next().unwrap_or(rel);
    leaf == "data.db"
        || leaf.starts_with("data.db-")
        || leaf.ends_with(".db")
        || leaf.ends_with(".db-wal")
        || leaf.ends_with(".db-shm")
        || rel.starts_with(".co/")
        || rel == co::manifest::MANIFEST_FILENAME
        || leaf.starts_with('.') // dotfiles (.gitkeep, .DS_Store, …)
}

/// Collect content-file paths under `root`, relative to `root`, skipping
/// bookkeeping files and the per-universe `data.db` family.
fn walk_content(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(base, &path, out);
        } else if let Ok(rel) = path.strip_prefix(base)
            && let Some(rel) = rel.to_str()
            && !is_bookkeeping(rel)
        {
            out.push(rel.to_string());
        }
    }
}

fn indexed_paths(storage: &Storage, key: &str) -> HashSet<String> {
    let arc = storage.universe_conn(key);
    let conn = arc.lock().expect("universe conn lock");
    let mut set = HashSet::new();
    if let Ok(mut stmt) = conn.prepare("SELECT path FROM entries WHERE universe_key = ?1")
        && let Ok(rows) = stmt.query_map([key], |r| r.get::<_, String>(0))
    {
        for p in rows.flatten() {
            set.insert(p);
        }
    }
    set
}

fn universe_keys(storage: &Storage) -> Vec<String> {
    let mut keys = Vec::new();
    if let Ok(mut stmt) = storage
        .conn()
        .prepare("SELECT key FROM universes ORDER BY key")
        && let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0))
    {
        for k in rows.flatten() {
            keys.push(k);
        }
    }
    keys
}

fn main() {
    let data_dir = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("CO_DATA_DIR").ok())
        .unwrap_or_else(|| "data".to_string());
    let data_dir = PathBuf::from(data_dir);

    if !data_dir.join("meta.db").exists() {
        eprintln!(
            "audit_serve: {} has no meta.db — pass a CO data dir as the first argument",
            data_dir.display()
        );
        std::process::exit(2);
    }

    let storage = Storage::new(&data_dir);
    let keys = universe_keys(&storage);

    println!(
        "audit_serve (CO-439): {} · {} universe(s)\n",
        data_dir.display(),
        keys.len()
    );

    let mut total_leaks = 0usize;
    for key in &keys {
        let root = storage.universe_root(key);
        if !root.exists() {
            continue;
        }
        let on_disk = walk_content(&root);
        let indexed = indexed_paths(&storage, key);
        let leaks = unindexed_disk_paths(&indexed, &on_disk);
        if leaks.is_empty() {
            continue;
        }
        total_leaks += leaks.len();
        println!(
            "  [{}] {} servable-but-not-indexed file(s):",
            key,
            leaks.len()
        );
        for p in &leaks {
            println!("      • {p}");
        }
    }

    if total_leaks == 0 {
        println!("  no leak surface — every on-disk content file is in the index.");
        std::process::exit(0);
    }

    println!(
        "\naudit_serve: {total_leaks} file(s) on disk but absent from the index. \
         The serve allowlist (CO-439) refuses these; remove them or index them via \
         the drafts→published flow."
    );
    std::process::exit(1);
}
