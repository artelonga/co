// CO-271: smoke test — reseed_co_root_files seeds LICENSE (bare filename, no
// extension) as LICENSE.md into the co universe entry index.
use tempfile::tempdir;

use co_web::entry_index::EntryIndex;
use co_web::storage::Storage;

fn make_storage_with_co(data_dir: &std::path::Path) -> Storage {
    let mut s = Storage::new(data_dir);
    s.seed_admin_content_universes();
    s
}

#[test]
fn test_license_entry_seeded_from_bare_filename() {
    let data_dir = tempdir().unwrap();
    let root_dir = tempdir().unwrap();

    std::fs::write(
        root_dir.path().join("LICENSE"),
        "GNU AFFERO GENERAL PUBLIC LICENSE\nVersion 3, 19 November 2007\n",
    )
    .unwrap();

    let mut storage = make_storage_with_co(data_dir.path());
    storage.reseed_co_root_files(root_dir.path());

    let co_uc = storage.universe_pool.get_or_open("co");
    let uc_guard = co_uc.lock().unwrap();
    let idx = EntryIndex::new(&uc_guard);

    let entry = idx.get("co", "LICENSE.md").unwrap();
    assert!(entry.is_some(), "LICENSE entry must exist as LICENSE.md");
    let entry = entry.unwrap();
    assert_eq!(entry.entry_type, "page");
    assert!(
        entry.body.contains("AFFERO") || entry.body.contains("GNU"),
        "body must contain AGPL text"
    );
}

#[test]
fn test_changelog_and_readme_not_regressed() {
    let data_dir = tempdir().unwrap();
    let root_dir = tempdir().unwrap();

    std::fs::write(
        root_dir.path().join("CHANGELOG.md"),
        "# Changelog\n## v1.0\n- initial\n",
    )
    .unwrap();
    std::fs::write(root_dir.path().join("README.md"), "# CO\nOpen source.\n").unwrap();
    std::fs::write(
        root_dir.path().join("LICENSE"),
        "GNU AFFERO GENERAL PUBLIC LICENSE\n",
    )
    .unwrap();

    let mut storage = make_storage_with_co(data_dir.path());
    storage.reseed_co_root_files(root_dir.path());

    let co_uc = storage.universe_pool.get_or_open("co");
    let uc_guard = co_uc.lock().unwrap();
    let idx = EntryIndex::new(&uc_guard);

    assert!(
        idx.get("co", "CHANGELOG.md").unwrap().is_some(),
        "CHANGELOG.md must be seeded"
    );
    assert!(
        idx.get("co", "README.md").unwrap().is_some(),
        "README.md must be seeded"
    );
    assert!(
        idx.get("co", "LICENSE.md").unwrap().is_some(),
        "LICENSE.md must be seeded"
    );
}
