use super::super::*;
use super::support::*;
use co::manifest::{ContentType, Manifest, Presentation};
use std::collections::BTreeMap;

// ---- extract_body_wikilinks (CO-363) — covers all 5 acceptance forms ----

/// Form 1: cross-universe wikilink without label.
#[test]
fn test_body_wikilinks_cross_universe_extracted() {
    let body = "See [[mbya::terms/jaxy-jatere]] and also [[yoruba::iya]].";
    let rels = extract_body_wikilinks(body);
    assert_eq!(rels.len(), 2);
    assert!(rels.contains(&(
        "wikilink".to_string(),
        "terms/jaxy-jatere".to_string(),
        Some("mbya".to_string()),
        None,
    )));
    assert!(rels.contains(&(
        "wikilink".to_string(),
        "iya".to_string(),
        Some("yoruba".to_string()),
        None,
    )));
}

/// Form 2: cross-universe wikilink WITH label — link_text is preserved.
#[test]
fn test_body_wikilinks_cross_universe_with_label() {
    let body = "Reference [[concepts::mother.md|mãe]] here.";
    let rels = extract_body_wikilinks(body);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].0, "wikilink");
    assert_eq!(rels[0].1, "mother.md");
    assert_eq!(rels[0].2, Some("concepts".to_string()));
    assert_eq!(rels[0].3, Some("mãe".to_string()));
}

/// Form 3: same-universe wikilink — to_universe = None (means same universe).
#[test]
fn test_body_wikilinks_same_universe_extracted() {
    let body = "See [[terms/jaryi.md]] and [[concepts/mother.md|Mother]].";
    let rels = extract_body_wikilinks(body);
    assert_eq!(rels.len(), 2);
    // to_universe = None means "same universe as the source entry"
    assert!(rels.contains(&(
        "wikilink".to_string(),
        "terms/jaryi.md".to_string(),
        None,
        None,
    )));
    assert!(rels.contains(&(
        "wikilink".to_string(),
        "concepts/mother.md".to_string(),
        None,
        Some("Mother".to_string()),
    )));
}

/// Form 4: deprecated relative-path wikilink.
#[test]
fn test_body_wikilinks_relative_deprecated() {
    let body = "See [[../sibling/x]] and [[./local/y]].";
    let rels = extract_body_wikilinks(body);
    assert_eq!(rels.len(), 2);
    assert!(rels.iter().all(|r| r.0 == "wikilink_relative_deprecated"));
    assert!(rels.iter().any(|r| r.1 == "../sibling/x"));
    assert!(rels.iter().any(|r| r.1 == "./local/y"));
}

#[test]
fn test_body_wikilinks_empty_body() {
    let rels = extract_body_wikilinks("");
    assert!(rels.is_empty());
}

#[test]
fn test_body_wikilinks_mixed_same_and_cross() {
    let body = "Links: [[local.md]] [[key::remote.md]] [[other::path/to/doc.md|Title]].";
    let rels = extract_body_wikilinks(body);
    assert_eq!(rels.len(), 3);
    // same-universe
    assert!(rels.contains(&("wikilink".to_string(), "local.md".to_string(), None, None,)));
    // cross-universe without label
    assert!(rels.contains(&(
        "wikilink".to_string(),
        "remote.md".to_string(),
        Some("key".to_string()),
        None,
    )));
    // cross-universe with label
    assert!(rels.contains(&(
        "wikilink".to_string(),
        "path/to/doc.md".to_string(),
        Some("other".to_string()),
        Some("Title".to_string()),
    )));
}

// ---- backfill_body_wikilinks ----

#[test]
fn test_backfill_body_wikilinks_inserts_cross_universe() {
    let conn = setup_db();
    conn.execute(
        "INSERT INTO entries (path, universe_key, entry_type, frontmatter_json, payload, body, body_hash)
         VALUES ('notes/a.md', 'mbya', 'nota',
                 '{}', '{}',
                 'See [[concepts::mother.md|mãe]] and [[terms/jaryi]].',
                 'abc')",
        [],
    )
    .unwrap();

    let n = backfill_body_wikilinks(&conn, "mbya").unwrap();
    assert_eq!(n, 1);

    let idx = RelationIndex::new(&conn);
    let rows = idx.outbound("mbya", "notes/a.md").unwrap();
    assert_eq!(rows.len(), 2);

    let cross: Vec<_> = rows.iter().filter(|r| r.to_universe.is_some()).collect();
    assert_eq!(cross.len(), 1);
    assert_eq!(cross[0].to_universe, Some("concepts".to_string()));
    assert_eq!(cross[0].to_path, "mother.md");
    assert_eq!(cross[0].link_text, Some("mãe".to_string()));

    let same: Vec<_> = rows.iter().filter(|r| r.to_universe.is_none()).collect();
    assert_eq!(same.len(), 1);
    assert_eq!(same[0].to_path, "terms/jaryi");
}

#[test]
fn test_backfill_no_affected_types_is_noop() {
    let conn = setup_db();
    // Manifest with no ref fields
    let manifest = Manifest {
        schema_version: 1,
        name: "X".to_string(),
        parent: None,
        surface_dns: None,
        visibility: None,
        content_types: vec![ContentType {
            name: "tarefa".to_string(),
            schema: BTreeMap::new(),
            presentation: Presentation::default(),
            indexes: vec![],
            changelog_summary: None,
        }],
        doc_generators: vec![],
        relationships: vec![],
        views: vec![],
        properties_per_type: std::collections::BTreeMap::new(),
    };
    let n = backfill_for_manifest(&conn, "u1", &manifest).unwrap();
    assert_eq!(n, 0);
}
