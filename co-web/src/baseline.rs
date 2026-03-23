use std::fs;
use std::path::Path;

use crate::storage::{Storage, seed_data};

/// Check if a data directory already has a database with project data.
pub fn has_data(data_dir: &str) -> bool {
    let db_path = Path::new(data_dir).join("co.db");
    if !db_path.exists() {
        return false;
    }
    let storage = Storage::new(data_dir);
    storage.has_data()
}

/// Seed the database with baseline projects and tasks.
pub fn seed_baseline(data_dir: &str) {
    let mut storage = Storage::new(data_dir);
    seed_data(&mut storage);
}

/// Reset local data to baseline (destructive).
pub fn reset_to_baseline(data_dir: &str) {
    let db_path = Path::new(data_dir).join("co.db");
    // Remove existing database files
    if db_path.exists() {
        fs::remove_file(&db_path).expect("Failed to remove database");
    }
    // Also remove WAL and SHM files if present
    let wal_path = db_path.with_extension("db-wal");
    let shm_path = db_path.with_extension("db-shm");
    let _ = fs::remove_file(wal_path);
    let _ = fs::remove_file(shm_path);

    seed_baseline(data_dir);
}

/// Export data to YAML files for backwards compatibility.
pub fn export_data(data_dir: &str, dest_dir: &str) -> anyhow::Result<()> {
    let storage = Storage::new(data_dir);
    let dest = Path::new(dest_dir);

    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;

    for project in storage.list_projects() {
        let project_dir = dest.join(project.key.to_lowercase());
        fs::create_dir_all(&project_dir)?;

        let project_yaml = serde_yaml::to_string(&project)?;
        fs::write(project_dir.join("project.yaml"), project_yaml)?;

        for task in storage.list_tasks(&project.key) {
            let filename = format!("{}-{}.md", project.key, task.id);
            let frontmatter = serde_yaml::to_string(&serde_json::json!({
                "id": task.id,
                "title": task.title,
                "status": task.status,
                "priority": task.priority,
                "due_date": task.due_date,
                "parent": task.parent,
                "labels": task.labels,
                "created_at": task.created_at,
                "updated_at": task.updated_at,
            }))?;
            let content = format!("---\n{}---\n\n{}\n", frontmatter, task.description);
            fs::write(project_dir.join(filename), content)?;
        }
    }

    println!("Exported data to {}", dest.display());
    Ok(())
}
