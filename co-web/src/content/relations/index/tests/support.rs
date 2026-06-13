//! Shared helpers for the relations index tests (CO-215 pattern).

use co::manifest::{
    Cardinality, ContentType, FieldDef, FieldType, Manifest, Presentation, Relationship,
};
use rusqlite::Connection;
use std::collections::BTreeMap;

// ---- DB helpers ----

pub fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE entries (
            path TEXT NOT NULL,
            universe_key TEXT NOT NULL,
            entry_type TEXT NOT NULL,
            title TEXT,
            frontmatter_json TEXT NOT NULL DEFAULT '{}',
            payload TEXT NOT NULL DEFAULT '{}',
            body TEXT NOT NULL DEFAULT '',
            body_hash TEXT NOT NULL DEFAULT '',
            created_at TEXT,
            updated_at TEXT,
            PRIMARY KEY (universe_key, path)
        );
        CREATE TABLE entry_relations (
            universe_key  TEXT NOT NULL,
            from_path     TEXT NOT NULL,
            to_path       TEXT NOT NULL,
            relation_type TEXT NOT NULL,
            created_at    TEXT NOT NULL,
            to_universe   TEXT,
            link_text     TEXT,
            PRIMARY KEY (universe_key, from_path, to_path, relation_type)
        );
        CREATE INDEX idx_er_from ON entry_relations(universe_key, from_path, relation_type);
        CREATE INDEX idx_er_to   ON entry_relations(universe_key, to_path,   relation_type);",
    )
    .unwrap();
    conn
}

pub fn evento_manifest() -> Manifest {
    let mut schema = BTreeMap::new();
    schema.insert(
        "attendees".to_string(),
        FieldDef {
            field_type: FieldType::RefList,
            required: false,
            values: vec![],
            semantic: None,
            target: Some("pessoa".to_string()),
        },
    );
    schema.insert(
        "title".to_string(),
        FieldDef {
            field_type: FieldType::String,
            required: true,
            values: vec![],
            semantic: None,
            target: None,
        },
    );
    Manifest {
        schema_version: 1,
        name: "Test".to_string(),
        content_types: vec![ContentType {
            name: "evento".to_string(),
            schema,
            presentation: Presentation::default(),
            indexes: vec![],
            changelog_summary: None,
        }],
        doc_generators: vec![],
        relationships: vec![Relationship {
            from: "evento.attendees".to_string(),
            to: "pessoa".to_string(),
            cardinality: Cardinality::ManyToMany,
        }],
        views: vec![],
        properties_per_type: std::collections::BTreeMap::new(),
    }
}

pub fn term_manifest() -> Manifest {
    let mut schema = BTreeMap::new();
    schema.insert(
        "concept".to_string(),
        FieldDef {
            field_type: FieldType::Ref,
            required: false,
            values: vec![],
            semantic: None,
            target: Some("concept".to_string()),
        },
    );
    schema.insert(
        "word".to_string(),
        FieldDef {
            field_type: FieldType::String,
            required: true,
            values: vec![],
            semantic: None,
            target: None,
        },
    );
    Manifest {
        schema_version: 1,
        name: "guarani-mbya".to_string(),
        content_types: vec![ContentType {
            name: "term".to_string(),
            schema,
            presentation: Presentation::default(),
            indexes: vec![],
            changelog_summary: None,
        }],
        properties_per_type: Default::default(),
        doc_generators: vec![],
        relationships: vec![],
        views: vec![],
    }
}
