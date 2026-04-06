//! `co migrate` — data migration commands.

use co_web::storage::Storage;

/// Migrate existing SQLite data (projects/tasks/comments) to .md files + entry index.
///
/// This runs the same migration that the web server executes on startup (v13),
/// but can be triggered manually from the CLI to pre-populate a vault directory.
pub fn run_entries(data_dir: &str) {
    println!("Migrating entries from SQLite → .md files ({data_dir})...");

    // Opening Storage triggers all migrations including v13 (migrate_old_data_to_entries).
    let storage = Storage::new(data_dir);
    let version = storage.schema_version();
    println!("Schema version: {version}");

    // Count resulting entries
    let conn = storage.conn();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))
        .unwrap_or(0);

    println!("Done. {count} entries indexed.");
    println!("Vault directory: {}/universes/", data_dir);
}
