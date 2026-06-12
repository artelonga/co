use super::support::*;
use crate::models::UpdateUniverseFormConfig;
use rusqlite::params;

#[test]
fn test_universe_form_config_defaults() {
    let (storage, _dir) = make_storage();
    let config = storage
        .get_universe_form_config("default")
        .expect("default universe must exist");
    assert_eq!(config.theme_preset, "scholarly-light");
    assert_eq!(config.layout, "board");
    assert!(config.font_headline.is_none());
    assert!(config.font_body.is_none());
    assert!(config.custom_tokens.is_none());
}

/// Updating theme_preset changes only that field; layout is preserved.
#[test]
fn test_update_form_config_theme() {
    let (mut storage, _dir) = make_storage();
    let updated = storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("relic-dark".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(updated.theme_preset, "relic-dark");
    assert_eq!(updated.layout, "board"); // unchanged

    // Persisted correctly.
    let persisted = storage.get_universe_form_config("default").unwrap();
    assert_eq!(persisted.theme_preset, "relic-dark");
}

/// Cloning a universe copies its form config exactly.
#[test]
fn test_clone_universe_inherits_form_config() {
    let (mut storage, _dir) = make_storage();

    // Give the default universe a custom theme + layout.
    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("scholarly-dark".to_string()),
                layout: Some("calendar".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

    // Make default public so it can be cloned.
    storage
        .conn()
        .execute(
            "UPDATE universes SET is_public = 1 WHERE key = 'default'",
            params![],
        )
        .unwrap();

    storage
        .clone_universe("default", "clone1", "Clone 1", "", "usr_test")
        .unwrap();

    let clone_config = storage
        .get_universe_form_config("clone1")
        .expect("clone must have form config");
    assert_eq!(clone_config.theme_preset, "scholarly-dark");
    assert_eq!(clone_config.layout, "calendar");
}

/// Changing form config does not affect entries in the same universe.
#[test]
fn test_form_config_change_does_not_affect_entries() {
    let (mut storage, _dir) = make_storage();

    // Create a project entry so entries table is non-empty.
    let universe_root = storage.universe_root("default");
    let entry = crate::entry_index::make_entry(
        "projects/TEST/_project.md",
        serde_json::json!({
            "type": "project",
            "key": "TEST",
            "title": "Test",
            "status": "active",
            "next_id": 1,
            "archived": false,
            "tags": []
        }),
        "Test project",
    );
    co::entry::write_entry(&universe_root, &entry).unwrap();
    crate::entry_index::EntryIndex::new(storage.conn())
        .upsert("default", &entry)
        .unwrap();

    // Change theme.
    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("relic".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

    // Entry still present and unmodified.
    let index = crate::entry_index::EntryIndex::new(storage.conn());
    let count = index.count("default", Some("project"));
    assert!(
        count > 0,
        "project entries must still be present after config change"
    );

    // Config changed.
    let config = storage.get_universe_form_config("default").unwrap();
    assert_eq!(config.theme_preset, "relic");
}

/// `.universo.yaml` is written when form config is updated.
#[test]
fn test_universo_yaml_written_on_update() {
    let (mut storage, _dir) = make_storage();

    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("relic-light".to_string()),
                layout: Some("table".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

    let yaml_path = storage.universe_root("default").join(".universo.yaml");
    assert!(yaml_path.exists(), ".universo.yaml must be written");
    let contents = std::fs::read_to_string(yaml_path).unwrap();
    assert!(contents.contains("relic-light"));
    assert!(contents.contains("table"));
}
