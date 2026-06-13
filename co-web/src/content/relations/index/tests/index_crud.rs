use super::super::*;
use super::support::*;

// ---- RelationIndex ----

#[test]
fn test_replace_all_inserts_rows() {
    let conn = setup_db();
    let idx = RelationIndex::new(&conn);
    idx.replace_all(
        "u1",
        "events/hackathon.md",
        &[
            (
                "attendees".to_string(),
                "pessoas/yuri.md".to_string(),
                None,
                None,
            ),
            (
                "attendees".to_string(),
                "pessoas/ana.md".to_string(),
                None,
                None,
            ),
        ],
    )
    .unwrap();

    let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_replace_all_idempotent() {
    let conn = setup_db();
    let idx = RelationIndex::new(&conn);
    let rels = [(
        "attendees".to_string(),
        "pessoas/yuri.md".to_string(),
        None,
        None,
    )];
    idx.replace_all("u1", "events/hackathon.md", &rels).unwrap();
    idx.replace_all("u1", "events/hackathon.md", &rels).unwrap();

    let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
    assert_eq!(rows.len(), 1, "duplicate replace must not add rows");
}

#[test]
fn test_replace_all_removes_stale_relations() {
    let conn = setup_db();
    let idx = RelationIndex::new(&conn);
    idx.replace_all(
        "u1",
        "events/hackathon.md",
        &[
            (
                "attendees".to_string(),
                "pessoas/yuri.md".to_string(),
                None,
                None,
            ),
            (
                "attendees".to_string(),
                "pessoas/ana.md".to_string(),
                None,
                None,
            ),
        ],
    )
    .unwrap();
    // Update: only yuri remains
    idx.replace_all(
        "u1",
        "events/hackathon.md",
        &[(
            "attendees".to_string(),
            "pessoas/yuri.md".to_string(),
            None,
            None,
        )],
    )
    .unwrap();

    let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].to_path, "pessoas/yuri.md");
}

#[test]
fn test_delete_for_entry() {
    let conn = setup_db();
    let idx = RelationIndex::new(&conn);
    idx.replace_all(
        "u1",
        "events/hackathon.md",
        &[(
            "attendees".to_string(),
            "pessoas/yuri.md".to_string(),
            None,
            None,
        )],
    )
    .unwrap();
    idx.delete_for_entry("u1", "events/hackathon.md").unwrap();

    let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
    assert!(rows.is_empty());
}

#[test]
fn test_inbound_lookup() {
    let conn = setup_db();
    let idx = RelationIndex::new(&conn);
    idx.replace_all(
        "u1",
        "events/hackathon.md",
        &[(
            "attendees".to_string(),
            "pessoas/yuri.md".to_string(),
            None,
            None,
        )],
    )
    .unwrap();
    idx.replace_all(
        "u1",
        "events/conf.md",
        &[(
            "attendees".to_string(),
            "pessoas/yuri.md".to_string(),
            None,
            None,
        )],
    )
    .unwrap();

    let inbound = idx.inbound("u1", "pessoas/yuri.md").unwrap();
    assert_eq!(inbound.len(), 2);
    let from_paths: Vec<&str> = inbound.iter().map(|r| r.from_path.as_str()).collect();
    assert!(from_paths.contains(&"events/hackathon.md"));
    assert!(from_paths.contains(&"events/conf.md"));
}

#[test]
fn test_cross_universe_inbound_via_inbound_from_other() {
    let conn = setup_db();
    let idx = RelationIndex::new(&conn);

    idx.replace_all(
        "guarani-mbya",
        "terms/jaryi.md",
        &[(
            "concept".to_string(),
            "mother.md".to_string(),
            Some("concepts".to_string()),
            None,
        )],
    )
    .unwrap();

    let inbound = idx.inbound_from_other("concepts", "mother.md").unwrap();
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0].universe_key, "guarani-mbya");
    assert_eq!(inbound[0].from_path, "terms/jaryi.md");
    assert_eq!(inbound[0].relation_type, "concept");
    assert_eq!(inbound[0].to_universe, Some("concepts".to_string()));
    assert_eq!(inbound[0].to_path, "mother.md");
}

#[test]
fn test_cross_universe_concept_mother_inbound() {
    let conn = setup_db();
    let idx = RelationIndex::new(&conn);

    for (universe, term_path) in [
        ("guarani-mbya", "terms/jaryi.md"),
        ("portuguese", "terms/mae.md"),
        ("yoruba", "terms/iya.md"),
    ] {
        idx.replace_all(
            universe,
            term_path,
            &[(
                "concept".to_string(),
                "mother.md".to_string(),
                Some("concepts".to_string()),
                None,
            )],
        )
        .unwrap();
    }

    let inbound = idx.inbound_from_other("concepts", "mother.md").unwrap();
    assert_eq!(inbound.len(), 3, "three language planes point to mother");

    let from_universes: Vec<&str> = inbound.iter().map(|r| r.universe_key.as_str()).collect();
    assert!(from_universes.contains(&"guarani-mbya"));
    assert!(from_universes.contains(&"portuguese"));
    assert!(from_universes.contains(&"yoruba"));
    assert!(inbound.iter().all(|r| r.relation_type == "concept"));
}

#[test]
fn test_replace_all_stores_link_text() {
    let conn = setup_db();
    let idx = RelationIndex::new(&conn);
    idx.replace_all(
        "u1",
        "notes/a.md",
        &[(
            "wikilink".to_string(),
            "mother.md".to_string(),
            Some("concepts".to_string()),
            Some("mãe".to_string()),
        )],
    )
    .unwrap();

    let rows = idx.outbound("u1", "notes/a.md").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].link_text, Some("mãe".to_string()));
    assert_eq!(rows[0].to_universe, Some("concepts".to_string()));
}

// ---- backfill ----

#[test]
fn test_backfill_for_manifest_creates_relations() {
    let conn = setup_db();
    conn.execute(
        "INSERT INTO entries (path, universe_key, entry_type, frontmatter_json, payload, body_hash)
         VALUES ('events/hackathon.md', 'u1', 'evento',
                 '{\"attendees\":[\"pessoas/yuri.md\"],\"title\":\"Hack\"}',
                 '{\"attendees\":[\"pessoas/yuri.md\"],\"title\":\"Hack\"}',
                 'abc')",
        [],
    )
    .unwrap();

    let manifest = evento_manifest();
    let n = backfill_for_manifest(&conn, "u1", &manifest).unwrap();
    assert_eq!(n, 1, "one entry should be processed");

    let idx = RelationIndex::new(&conn);
    let rows = idx.outbound("u1", "events/hackathon.md").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].to_path, "pessoas/yuri.md");
    assert_eq!(rows[0].relation_type, "attendees");
    assert_eq!(rows[0].to_universe, None);
}

#[test]
fn test_backfill_for_manifest_co_uri_populates_to_universe() {
    let conn = setup_db();
    conn.execute(
        "INSERT INTO entries (path, universe_key, entry_type, frontmatter_json, payload, body_hash)
         VALUES ('terms/jaryi.md', 'guarani-mbya', 'term',
                 '{\"concept\":\"co://concepts/mother.md\",\"word\":\"jaryi\"}',
                 '{\"concept\":\"co://concepts/mother.md\",\"word\":\"jaryi\"}',
                 'abc')",
        [],
    )
    .unwrap();

    let manifest = term_manifest();
    let n = backfill_for_manifest(&conn, "guarani-mbya", &manifest).unwrap();
    assert_eq!(n, 1);

    let idx = RelationIndex::new(&conn);
    let rows = idx.outbound("guarani-mbya", "terms/jaryi.md").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].to_path, "mother.md");
    assert_eq!(rows[0].to_universe, Some("concepts".to_string()));
}
