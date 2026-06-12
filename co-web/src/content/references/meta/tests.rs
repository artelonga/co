use super::*;
use rusqlite::Connection;
use serde_json::json;

fn open_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .unwrap();
    conn.execute_batch(crate::universe_pool::UNIVERSE_SCHEMA_FOR_TEST)
        .unwrap();
    crate::universe_pool::run_universe_migrations_for_test(&conn);
    conn
}

#[test]
fn test_upsert_reference_meta_basic() {
    let conn = open_test_db();
    let fm = json!({
        "type": "reference",
        "medium": "pdf",
        "mime": "application/pdf",
        "seed_status": "stub",
    });
    upsert_reference_meta(
        &conn,
        "mbya",
        "refs/dooley.md",
        &fm,
        "some body text",
        Some("Dooley 2006"),
        std::path::Path::new("/nonexistent"),
    );

    let row: (String, String, String) = conn
        .query_row(
            "SELECT medium, mime, seed_status FROM references_meta \
             WHERE universe_key = 'mbya' AND entry_path = 'refs/dooley.md'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, "pdf");
    assert_eq!(row.1, "application/pdf");
    assert_eq!(row.2, "stub");
}

#[test]
fn test_upsert_enforces_stub_when_file_absent() {
    let conn = open_test_db();
    let fm = json!({
        "type": "reference",
        "medium": "pdf",
        "file": "missing.pdf",
        "seed_status": "reviewed",   // claimed reviewed, but file doesn't exist
    });
    upsert_reference_meta(
        &conn,
        "mbya",
        "refs/missing.md",
        &fm,
        "",
        None,
        std::path::Path::new("/nonexistent"),
    );
    let status: String = conn
        .query_row(
            "SELECT seed_status FROM references_meta WHERE entry_path = 'refs/missing.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "stub",
        "seed_status should be enforced to stub when blob absent"
    );
}

#[test]
fn test_remove_reference_meta_is_idempotent() {
    let conn = open_test_db();
    // Remove from an empty table — must not panic or error.
    remove_reference_meta(&conn, "mbya", "refs/nonexistent.md");

    // Insert then remove.
    let fm = json!({ "type": "reference", "medium": "pdf", "seed_status": "stub" });
    upsert_reference_meta(
        &conn,
        "mbya",
        "refs/a.md",
        &fm,
        "",
        None,
        std::path::Path::new("/x"),
    );
    remove_reference_meta(&conn, "mbya", "refs/a.md");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM references_meta WHERE entry_path = 'refs/a.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_upsert_is_idempotent() {
    let conn = open_test_db();
    let fm = json!({ "type": "reference", "medium": "video", "seed_status": "stub" });
    // Two identical upserts must leave exactly one row.
    upsert_reference_meta(
        &conn,
        "u",
        "r/a.md",
        &fm,
        "",
        None,
        std::path::Path::new("/x"),
    );
    upsert_reference_meta(
        &conn,
        "u",
        "r/a.md",
        &fm,
        "",
        None,
        std::path::Path::new("/x"),
    );
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM references_meta", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn test_references_tables_exist_after_migration() {
    let conn = open_test_db();
    let has_meta: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='references_meta'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(has_meta, "references_meta table should exist");

    let has_fts: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='reference_cards_fts'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(has_fts, "reference_cards_fts table should exist");
}

#[test]
fn test_schema_version_reaches_8() {
    let conn = open_test_db();
    let v: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap_or(0);
    assert!(
        v >= 8,
        "schema_version should be at least 8 (CO-158), got {v}"
    );
}

// --- CO-158 tests ---

#[test]
fn test_three_editions_round_trip() {
    let conn = open_test_db();
    let fm = json!({
        "type": "reference",
        "work_id": "ayvu-rapyta-cadogan-1959",
        "medium": "pdf",
        "seed_status": "stub",
        "primary_source_chain": [
            {"layer": 0, "role": "phenomenon"},
            {"layer": 1, "role": "transcription"},
            {"layer": 2, "role": "publication"},
        ],
        "editions": [
            {"edition_id": "usp-1959",    "seed_status": "native-confirmed"},
            {"edition_id": "ucsa-1992",   "seed_status": "reviewed"},
            {"edition_id": "ocr-2010",    "seed_status": "stub", "url": "https://example.com"},
        ],
    });

    upsert_reference_meta(
        &conn,
        "mbya",
        "refs/ayvu-rapyta.md",
        &fm,
        "body text",
        Some("Ayvu Rapyta"),
        std::path::Path::new("/nonexistent"),
    );

    // All 3 editions must be present.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM references_meta \
             WHERE universe_key = 'mbya' AND entry_path = 'refs/ayvu-rapyta.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 3, "expected 3 edition rows in references_meta");

    // All share the same work_id.
    let work_id_count: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT work_id) FROM references_meta \
             WHERE entry_path = 'refs/ayvu-rapyta.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        work_id_count, 1,
        "all editions should share the same work_id"
    );

    let work_id: String = conn
        .query_row(
            "SELECT work_id FROM references_meta WHERE entry_path = 'refs/ayvu-rapyta.md' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(work_id, "ayvu-rapyta-cadogan-1959");

    // primary_layer = min layer in primary_source_chain = 0.
    let layer: Option<i64> = conn
        .query_row(
            "SELECT primary_layer FROM references_meta \
             WHERE entry_path = 'refs/ayvu-rapyta.md' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        layer,
        Some(0),
        "primary_layer should be min chain layer = 0"
    );
}

#[test]
fn test_work_id_derived_from_path_when_absent() {
    let conn = open_test_db();
    let fm = json!({
        "type": "reference",
        "medium": "pdf",
        "seed_status": "stub",
    });
    upsert_reference_meta(
        &conn,
        "u",
        "refs/GNDicLex.md",
        &fm,
        "",
        None,
        std::path::Path::new("/nonexistent"),
    );

    let work_id: String = conn
        .query_row(
            "SELECT work_id FROM references_meta WHERE entry_path = 'refs/GNDicLex.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(work_id, "GNDicLex", "work_id should be the filename stem");
}

#[test]
fn test_sha256_duplicate_skips_second_edition() {
    let conn = open_test_db();

    // Card A: explicit sha256 in editions (simulates computed from disk).
    let fm_a = json!({
        "type": "reference",
        "work_id": "shared-work",
        "medium": "pdf",
        "editions": [
            {"edition_id": "first", "sha256": "deadbeef01", "seed_status": "stub"},
        ],
    });
    upsert_reference_meta(
        &conn,
        "u",
        "refs/a.md",
        &fm_a,
        "",
        None,
        std::path::Path::new("/nonexistent"),
    );

    // Card B: same sha256, same work_id, different entry_path.
    let fm_b = json!({
        "type": "reference",
        "work_id": "shared-work",
        "medium": "pdf",
        "editions": [
            {"edition_id": "dup", "sha256": "deadbeef01", "seed_status": "stub"},
        ],
    });
    upsert_reference_meta(
        &conn,
        "u",
        "refs/b.md",
        &fm_b,
        "",
        None,
        std::path::Path::new("/nonexistent"),
    );

    // Only one row with that sha256 should exist.
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM references_meta WHERE blob_sha256 = 'deadbeef01'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 1,
        "duplicate sha256 must not create a second edition row"
    );
}

#[test]
fn test_work_id_filter_returns_all_editions() {
    let conn = open_test_db();
    let fm = json!({
        "type": "reference",
        "work_id": "my-work",
        "medium": "pdf",
        "editions": [
            {"edition_id": "ed1", "seed_status": "stub"},
            {"edition_id": "ed2", "seed_status": "reviewed"},
        ],
    });
    upsert_reference_meta(
        &conn,
        "u",
        "refs/card.md",
        &fm,
        "",
        None,
        std::path::Path::new("/nonexistent"),
    );

    let edition_ids: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT edition_id FROM references_meta \
                 WHERE work_id = 'my-work' ORDER BY edition_id",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };
    assert_eq!(edition_ids, ["ed1", "ed2"]);
}

#[test]
fn test_primary_layer_null_when_no_chain() {
    let conn = open_test_db();
    let fm = json!({
        "type": "reference",
        "medium": "web",
        "seed_status": "stub",
    });
    upsert_reference_meta(
        &conn,
        "u",
        "refs/nochain.md",
        &fm,
        "",
        None,
        std::path::Path::new("/nonexistent"),
    );

    let layer: Option<i64> = conn
        .query_row(
            "SELECT primary_layer FROM references_meta WHERE entry_path = 'refs/nochain.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        layer, None,
        "primary_layer should be null when no primary_source_chain"
    );
}

#[test]
fn test_editions_replaced_on_update() {
    let conn = open_test_db();

    // Initial write: 2 editions.
    let fm1 = json!({
        "type": "reference",
        "work_id": "w",
        "medium": "pdf",
        "editions": [
            {"edition_id": "a", "seed_status": "stub"},
            {"edition_id": "b", "seed_status": "stub"},
        ],
    });
    upsert_reference_meta(
        &conn,
        "u",
        "refs/r.md",
        &fm1,
        "",
        None,
        std::path::Path::new("/x"),
    );

    // Update: remove edition "a", add edition "c".
    let fm2 = json!({
        "type": "reference",
        "work_id": "w",
        "medium": "pdf",
        "editions": [
            {"edition_id": "b", "seed_status": "reviewed"},
            {"edition_id": "c", "seed_status": "stub"},
        ],
    });
    upsert_reference_meta(
        &conn,
        "u",
        "refs/r.md",
        &fm2,
        "",
        None,
        std::path::Path::new("/x"),
    );

    let edition_ids: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT edition_id FROM references_meta \
                 WHERE entry_path = 'refs/r.md' ORDER BY edition_id",
            )
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    };
    assert_eq!(
        edition_ids,
        ["b", "c"],
        "removed edition 'a' must be gone; 'c' must appear"
    );
}
