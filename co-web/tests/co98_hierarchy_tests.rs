//! CO-98: hierarchical universes — storage-level coverage.
//!
//! Exercises the `parent_key` round-trip through the storage API and the
//! `list_surface_nodes` tree-list helper that exposes parent → child
//! relationships for sidebar/surface tree building. Route-level coverage lives
//! in `co-web/src/content/universe/routes/tests/reparent.rs`; these tests pin
//! the persistence layer itself.

use co_web::models::CreateUniverse;
use co_web::storage::Storage;
use tempfile::tempdir;

#[test]
fn test_timeline_trio_parent_key_round_trips_through_storage() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    // Seeding the trio sets `parent_key = 'template'` (CO-98 data model).
    storage.seed_all_timeline_universes();

    // Round-trip: each child reads back nested under the template.
    for child in ["tempo", "humanity", "universo"] {
        let u = storage
            .get_universe(child)
            .unwrap_or_else(|| panic!("timeline universe {child} should be seeded"));
        assert_eq!(
            u.parent_key.as_deref(),
            Some("template"),
            "{child} should be nested under template",
        );
    }
}

#[test]
fn test_created_universe_is_top_level_and_surface_tree_lists_parents() {
    let dir = tempdir().unwrap();
    let mut storage = Storage::new(dir.path());

    // Owner FK requires a user row.
    let user = storage.create_user("owner@example.com", "Owner").unwrap();

    // A freshly-created universe is top-level (`parent_key = None`), and the
    // round-trip preserves that — existing flat universes stay parentless.
    let created = storage
        .create_universe(
            CreateUniverse {
                key: "solo".into(),
                name: "Solo".into(),
                description: "top-level".into(),
            },
            &user.id,
        )
        .unwrap();
    assert!(
        created.parent_key.is_none(),
        "new universe must be top-level"
    );
    assert!(
        storage.get_universe("solo").unwrap().parent_key.is_none(),
        "round-trip: top-level universe stays parentless",
    );

    // Tree-list helper: `list_surface_nodes` surfaces the parent relationships
    // used to build the hierarchy. The seeded trio reports parent = template;
    // the solo universe is a tree root.
    storage.seed_all_timeline_universes();
    let nodes = storage.list_surface_nodes();
    for child in ["tempo", "humanity", "universo"] {
        let node = nodes
            .iter()
            .find(|n| n.key == child)
            .unwrap_or_else(|| panic!("{child} should appear in the surface tree"));
        assert_eq!(
            node.parent.as_deref(),
            Some("template"),
            "{child} should resolve under template in the tree-list helper",
        );
    }
    let solo = nodes
        .iter()
        .find(|n| n.key == "solo")
        .expect("solo universe should appear in the surface tree");
    assert!(solo.parent.is_none(), "solo must be a tree root");
}
