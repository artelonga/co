//! migrate_check — CO-376 (MVP): pre-prod migration validation.
//!
//! Usage:
//!   migrate_check <extracted-snapshot-dir>
//!
//! Applies the current binary's migrations against a **copy** of a staging
//! snapshot (an extracted `snapshot-*.tar.gz`: `meta.db` + `universes/.../data.db`)
//! and runs read-only smoke assertions on the post-migration shape. Never touches
//! a live database — point it at a throwaway extracted copy.
//!
//! Flow:
//!   1. Record a pre-migration baseline (raw connections, no migration run).
//!   2. `Storage::new(dir)` runs meta-db migrations (and the entry-split).
//!   3. Open each universe so per-universe pool migrations run.
//!   4. Assert the wave's tables/columns exist and are selectable, the yuri user
//!      survived, and the conserved entry total did not drift more than ±5%.
//!
//! Exit code: 0 iff every check passed, 1 if any failed, 2 on bad invocation.
//!
//! This is the CO-376 MVP. The auto-gating GitHub Action, the
//! `/api/v1/admin/migrations/snapshots` endpoint, and custom `migrations/<vN>.smoke.sql`
//! support are deferred to a CO-376 follow-up — see `docs/migration-checklist.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use co_web::storage::Storage;
use rusqlite::Connection;

struct Check {
    name: String,
    pass: bool,
    detail: String,
}

impl Check {
    fn new(name: impl Into<String>, pass: bool, detail: impl Into<String>) -> Self {
        Check {
            name: name.into(),
            pass,
            detail: detail.into(),
        }
    }
}

fn main() {
    let dir = match std::env::args().nth(1) {
        Some(d) => PathBuf::from(d),
        None => {
            eprintln!("usage: migrate_check <extracted-snapshot-dir>");
            std::process::exit(2);
        }
    };

    let meta_db = dir.join("meta.db");
    if !meta_db.exists() {
        eprintln!(
            "error: {} not found — expected an extracted snapshot dir (meta.db + universes/)",
            meta_db.display()
        );
        std::process::exit(2);
    }

    // Discover per-universe data.db files; the universe key is the leaf dir name
    // (`universes/{l1}/{l2}/{key}/data.db`).
    let universe_dbs = discover_universe_dbs(&dir.join("universes"));
    println!(
        "migrate_check: {} · {} universe data.db file(s)",
        dir.display(),
        universe_dbs.len()
    );

    // ── 1. Pre-migration baseline (raw, no migration) ───────────────────────
    let pre_meta_ver = raw_count(
        &meta_db,
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
    );
    let pre_users = raw_count(&meta_db, "SELECT COUNT(*) FROM users");
    let pre_universes = raw_count(&meta_db, "SELECT COUNT(*) FROM universes");
    let pre_meta_entries = raw_count(&meta_db, "SELECT COUNT(*) FROM entries");
    let pre_univ_entries: HashMap<String, i64> = universe_dbs
        .iter()
        .map(|(key, path)| (key.clone(), raw_count(path, "SELECT COUNT(*) FROM entries")))
        .collect();
    let pre_entry_total: i64 = pre_meta_entries + pre_univ_entries.values().sum::<i64>();

    // ── 2. Apply meta migrations (+ entry-split) ─────────────────────────────
    let storage = Storage::new(&dir);

    // ── 3. Apply per-universe pool migrations (first open runs them) ─────────
    for (key, _) in &universe_dbs {
        let _ = storage.universe_conn(key);
    }

    // ── 4. Smoke checks ──────────────────────────────────────────────────────
    let mut checks: Vec<Check> = Vec::new();
    let meta = storage.conn();

    let post_meta_ver: i64 = meta
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    checks.push(Check::new(
        "meta schema_version advanced",
        post_meta_ver >= pre_meta_ver,
        format!("v{pre_meta_ver} → v{post_meta_ver}"),
    ));

    checks.push(selectable(
        meta,
        "universes.source_kind/source_url/source_last_event_at (v68)",
        "SELECT source_kind, source_url, source_last_event_at FROM universes LIMIT 1",
    ));
    checks.push(table_exists(
        meta,
        "bridge_state table (v65)",
        "bridge_state",
    ));
    checks.push(table_exists(
        meta,
        "sync_conflicts table (v67)",
        "sync_conflicts",
    ));

    let yuri = meta
        .query_row(
            "SELECT COUNT(*) FROM users WHERE email = 'yuri@artelonga.com.br'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0);
    checks.push(Check::new(
        "yuri admin user survived migration",
        yuri >= 1,
        format!("{yuri} row(s)"),
    ));

    // Per-universe: pool migrations + the v17 source_marker column.
    let mut post_univ_entries: HashMap<String, i64> = HashMap::new();
    for (key, _) in &universe_dbs {
        let arc = storage.universe_conn(key);
        let conn = arc.lock().expect("universe conn lock");
        let pv: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_version",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        checks.push(Check::new(
            format!("[{key}] pool schema_version present"),
            pv >= 1,
            format!("v{pv}"),
        ));
        checks.push(selectable(
            &conn,
            format!("[{key}] entries.source_marker (pool v17)"),
            "SELECT source_marker FROM entries LIMIT 1",
        ));
        let cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
            .unwrap_or(0);
        post_univ_entries.insert(key.clone(), cnt);
    }

    // Counts that additive migrations must not change.
    let post_users: i64 = meta
        .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
        .unwrap_or(-1);
    let post_universes: i64 = meta
        .query_row("SELECT COUNT(*) FROM universes", [], |r| r.get(0))
        .unwrap_or(-1);
    checks.push(Check::new(
        "user count unchanged",
        post_users == pre_users,
        format!("{pre_users} → {post_users}"),
    ));
    checks.push(Check::new(
        "universe count unchanged",
        post_universes == pre_universes,
        format!("{pre_universes} → {post_universes}"),
    ));

    // Conserved entry total (the entry-split moves rows meta→universe; pool v17 is
    // additive) must stay within ±5%.
    let post_meta_entries: i64 = meta
        .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
        .unwrap_or(0);
    let post_entry_total: i64 = post_meta_entries + post_univ_entries.values().sum::<i64>();
    checks.push(drift_check(
        "entry total (meta + universes)",
        pre_entry_total,
        post_entry_total,
    ));

    // ── 5. Report ─────────────────────────────────────────────────────────────
    println!("\n  RESULT  CHECK");
    println!("  ──────  ───────────────────────────────────────────────────────");
    let mut failed = 0usize;
    for c in &checks {
        let mark = if c.pass { "PASS" } else { "FAIL" };
        if !c.pass {
            failed += 1;
        }
        println!("  [{mark}]  {} — {}", c.name, c.detail);
    }
    println!(
        "\nmigrate_check: {} passed, {} failed (of {})",
        checks.len() - failed,
        failed,
        checks.len()
    );

    if failed > 0 {
        std::process::exit(1);
    }
}

/// Recursively find every `data.db` under `root`; key = its parent directory name.
fn discover_universe_dbs(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn walk(dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("data.db")
            && let Some(key) = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
        {
            out.push((key.to_string(), path.clone()));
        }
    }
}

/// Open a read-only-ish raw connection and run a `COUNT`/scalar query; missing
/// tables or open failures return 0 (pre-migration shape may lack a table).
fn raw_count(db: &Path, sql: &str) -> i64 {
    let Ok(conn) = Connection::open(db) else {
        return 0;
    };
    conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0)
}

fn selectable(conn: &Connection, name: impl Into<String>, sql: &str) -> Check {
    match conn.prepare(sql).and_then(|mut s| {
        let mut rows = s.query([])?;
        rows.next()?; // executing the query is what proves the columns exist
        Ok(())
    }) {
        Ok(()) => Check::new(name, true, "selectable"),
        Err(e) => Check::new(name, false, format!("query failed: {e}")),
    }
}

fn table_exists(conn: &Connection, name: impl Into<String>, table: &str) -> Check {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Check::new(
        name,
        exists >= 1,
        if exists >= 1 { "present" } else { "MISSING" }.to_string(),
    )
}

/// Pass if `post` is within ±5% of `pre` (additive migrations should not lose rows).
fn drift_check(name: impl Into<String>, pre: i64, post: i64) -> Check {
    let pass = if pre == 0 {
        post == 0
    } else {
        let lo = (pre as f64 * 0.95).floor() as i64;
        let hi = (pre as f64 * 1.05).ceil() as i64;
        post >= lo && post <= hi
    };
    Check::new(name, pass, format!("{pre} → {post} (±5%)"))
}
