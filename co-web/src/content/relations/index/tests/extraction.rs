use super::super::*;
use super::support::*;
use serde_json::json;

// ---- parse_co_uri ----

#[test]
fn test_parse_co_uri_cross_universe() {
    let cr = parse_co_uri("co://concepts/mother.md").unwrap();
    assert_eq!(cr.universe, Some("concepts".to_string()));
    assert_eq!(cr.path, "mother.md");
}

#[test]
fn test_parse_co_uri_cross_universe_subpath() {
    let cr = parse_co_uri("co://concepts/concepts/mother.md").unwrap();
    assert_eq!(cr.universe, Some("concepts".to_string()));
    assert_eq!(cr.path, "concepts/mother.md");
}

#[test]
fn test_parse_co_uri_plain_path() {
    let cr = parse_co_uri("terms/xy.md").unwrap();
    assert_eq!(cr.universe, None);
    assert_eq!(cr.path, "terms/xy.md");
}

#[test]
fn test_parse_co_uri_malformed_returns_none() {
    // No slash after universe component
    assert!(parse_co_uri("co://noslash").is_none());
}

/// CO-363: frontmatter `key::path` syntax populates to_universe.
#[test]
fn test_parse_co_uri_key_double_colon_path() {
    let cr = parse_co_uri("yoruba::terms/ogunte").unwrap();
    assert_eq!(cr.universe, Some("yoruba".to_string()));
    assert_eq!(cr.path, "terms/ogunte");
}

#[test]
fn test_parse_co_uri_key_double_colon_file() {
    let cr = parse_co_uri("concepts::mother.md").unwrap();
    assert_eq!(cr.universe, Some("concepts".to_string()));
    assert_eq!(cr.path, "mother.md");
}

// ---- extract_relations ----

#[test]
fn test_extract_ref_list_creates_multiple_relations() {
    let manifest = evento_manifest();
    let fm = json!({"attendees": ["pessoas/yuri.md", "pessoas/ana.md"], "title": "Hackathon"});
    let rels = extract_relations(&manifest, "evento", &fm);
    assert_eq!(rels.len(), 2);
    assert!(rels.contains(&(
        "attendees".to_string(),
        "pessoas/yuri.md".to_string(),
        None,
        None
    )));
    assert!(rels.contains(&(
        "attendees".to_string(),
        "pessoas/ana.md".to_string(),
        None,
        None
    )));
}

#[test]
fn test_extract_ref_list_resolves_wikilinks() {
    let manifest = evento_manifest();
    let fm = json!({"attendees": ["[[pessoas/yuri.md]]", "[[pessoas/ana.md|Ana]]"], "title": "X"});
    let rels = extract_relations(&manifest, "evento", &fm);
    assert_eq!(rels.len(), 2);
    assert!(rels.contains(&(
        "attendees".to_string(),
        "pessoas/yuri.md".to_string(),
        None,
        None
    )));
    assert!(rels.contains(&(
        "attendees".to_string(),
        "pessoas/ana.md".to_string(),
        None,
        None
    )));
}

#[test]
fn test_extract_co_uri_populates_to_universe() {
    let manifest = term_manifest();
    let fm = json!({"concept": "co://concepts/mother.md", "word": "jaryi"});
    let rels = extract_relations(&manifest, "term", &fm);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].0, "concept");
    assert_eq!(rels[0].1, "mother.md");
    assert_eq!(rels[0].2, Some("concepts".to_string()));
    assert_eq!(rels[0].3, None); // no link_text for frontmatter refs
}

#[test]
fn test_extract_co_uri_subpath() {
    let manifest = term_manifest();
    let fm = json!({"concept": "co://concepts/concepts/mother.md", "word": "jaryi"});
    let rels = extract_relations(&manifest, "term", &fm);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].1, "concepts/mother.md");
    assert_eq!(rels[0].2, Some("concepts".to_string()));
}

/// CO-363: frontmatter `key::path` syntax populates `to_universe`.
#[test]
fn test_extract_frontmatter_key_double_colon_path() {
    let manifest = term_manifest();
    let fm = json!({"concept": "yoruba::terms/ogunte", "word": "iya"});
    let rels = extract_relations(&manifest, "term", &fm);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].0, "concept");
    assert_eq!(rels[0].1, "terms/ogunte");
    assert_eq!(rels[0].2, Some("yoruba".to_string()));
    assert_eq!(rels[0].3, None);
}

#[test]
fn test_extract_string_field_no_relation() {
    let manifest = evento_manifest();
    // "title" is a plain string field — wikilinks in it must not create relations
    let fm = json!({"attendees": [], "title": "[[some wikilink]]"});
    let rels = extract_relations(&manifest, "evento", &fm);
    assert!(rels.is_empty(), "non-ref field must not produce relations");
}

#[test]
fn test_extract_unknown_type_returns_empty() {
    let manifest = evento_manifest();
    let fm = json!({"anything": "value"});
    let rels = extract_relations(&manifest, "unknown_type", &fm);
    assert!(rels.is_empty());
}

// ---- CO-418: provenance / traceback relations ----

#[test]
fn test_extract_provenance_relations_source_and_requested_by() {
    let fm = json!({
        "type": "page",
        "source": "github:yurisugano/SensorySpeech@cafe1234",
        "source_path": "docs/intro.md",
        "source_kind": "github",
        "requested_by": "CO-419",
    });
    let rels = extract_provenance_relations(&fm);
    // origin → source ; requested_by → task
    let origin = rels
        .iter()
        .find(|r| r.0 == "origin")
        .expect("origin relation present");
    assert_eq!(origin.1, "github:yurisugano/SensorySpeech@cafe1234");
    assert_eq!(origin.2, Some("@source".to_string()));
    let req = rels
        .iter()
        .find(|r| r.0 == "requested_by")
        .expect("requested_by relation present");
    assert_eq!(req.1, "CO-419");
    assert_eq!(req.2, Some("@task".to_string()));
}

#[test]
fn test_extract_provenance_relations_absent_fields() {
    let fm = json!({ "type": "note", "title": "no provenance" });
    let rels = extract_provenance_relations(&fm);
    assert!(rels.is_empty(), "no source/requested_by ⇒ no edges");
}

#[test]
fn test_sync_includes_provenance_edges() {
    let conn = setup_db();
    let fm = json!({
        "type": "page",
        "source": "github:foo/bar@abc",
        "requested_by": "CO-418",
    });
    // No manifest, empty body ⇒ only provenance edges contribute.
    let count = sync_entry_relations(&conn, "u1", "docs/x.md", "page", &fm, "", None).unwrap();
    assert_eq!(count, 2, "origin + requested_by");
    let rows = RelationIndex::new(&conn)
        .outbound("u1", "docs/x.md")
        .unwrap();
    let types: Vec<&str> = rows.iter().map(|r| r.relation_type.as_str()).collect();
    assert!(types.contains(&"origin"));
    assert!(types.contains(&"requested_by"));
}
