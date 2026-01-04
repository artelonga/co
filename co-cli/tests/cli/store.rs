//! Archive command tests (Archive & Storage)
//!
//! Tests for co archive, co archive restore, and co archive list commands.

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

// ============================================================================
// Archive Tests
// ============================================================================

#[test]
fn test_archive_moves_to_archive_dir() {
    let tmp = tempdir().unwrap();

    // Create space with task
    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("tasks")).unwrap();
    std::fs::write(
        work_path.join("tasks/task-12.1.1.md"),
        "---\ntype: task\nid: 12.1.1\nstatus: done\n---\n\n# Completed Task\n",
    )
    .unwrap();

    // Archive the task
    co_command()
        .current_dir(tmp.path())
        .args(["archive", "task-12.1.1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Archived"));

    // Original should be gone
    assert!(!work_path.join("tasks/task-12.1.1.md").exists());

    // Archive should exist with mirrored structure
    assert!(work_path.join("archive/tasks/task-12.1.1.md").exists());
}

#[test]
fn test_archive_adds_archived_at_timestamp() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("tasks")).unwrap();
    std::fs::write(
        work_path.join("tasks/task-1.md"),
        "---\ntype: task\nid: 1\nstatus: done\n---\n\n# Task\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["archive", "task-1"])
        .assert()
        .success();

    let archived_content =
        std::fs::read_to_string(work_path.join("archive/tasks/task-1.md")).unwrap();
    assert!(archived_content.contains("archived_at:"));
}

#[test]
fn test_archive_adds_indexed_false_tag() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("tasks")).unwrap();
    std::fs::write(
        work_path.join("tasks/task-1.md"),
        "---\ntype: task\nid: 1\nstatus: done\n---\n\n# Task\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["archive", "task-1"])
        .assert()
        .success();

    let archived_content =
        std::fs::read_to_string(work_path.join("archive/tasks/task-1.md")).unwrap();
    // Deindex: mark as not indexed so co ignores these files
    assert!(archived_content.contains("indexed: false"));
}

#[test]
fn test_archive_preserves_directory_structure() {
    let tmp = tempdir().unwrap();

    // Create nested structure: work/definitions/term.md
    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("definitions")).unwrap();
    std::fs::write(
        work_path.join("definitions/term.md"),
        "---\ntype: definition\nid: term\n---\n\n# Term\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["archive", "term"])
        .assert()
        .success();

    // Should be in work/archive/definitions/term.md
    assert!(work_path.join("archive/definitions/term.md").exists());
}

#[test]
fn test_archive_warns_if_already_archived() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("tasks")).unwrap();
    std::fs::create_dir_all(work_path.join("archive/tasks")).unwrap();

    // Create original file
    std::fs::write(
        work_path.join("tasks/task-1.md"),
        "---\ntype: task\nid: 1\nstatus: done\n---\n\n# Task v2\n",
    )
    .unwrap();

    // Create existing archive
    std::fs::write(
        work_path.join("archive/tasks/task-1.md"),
        "---\ntype: task\nid: 1\nstatus: done\narchived_at: 2024-01-01T00:00:00Z\nindexed: false\n---\n\n# Task v1\n",
    )
    .unwrap();

    // Archive should warn and fail without --force
    co_command()
        .current_dir(tmp.path())
        .args(["archive", "task-1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists in archive"))
        .stderr(predicate::str::contains("--force"));
}

#[test]
fn test_archive_force_replaces_existing() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("tasks")).unwrap();
    std::fs::create_dir_all(work_path.join("archive/tasks")).unwrap();

    // Create original file with updated content
    std::fs::write(
        work_path.join("tasks/task-1.md"),
        "---\ntype: task\nid: 1\nstatus: done\n---\n\n# Task v2 Updated\n",
    )
    .unwrap();

    // Create existing archive with old content
    std::fs::write(
        work_path.join("archive/tasks/task-1.md"),
        "---\ntype: task\nid: 1\nstatus: done\narchived_at: 2024-01-01T00:00:00Z\nindexed: false\n---\n\n# Task v1 Old\n",
    )
    .unwrap();

    // Archive with --force should succeed
    co_command()
        .current_dir(tmp.path())
        .args(["archive", "task-1", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("replaced existing"));

    // Original should be removed
    assert!(!work_path.join("tasks/task-1.md").exists());

    // Archive should have new content
    let archived_content =
        std::fs::read_to_string(work_path.join("archive/tasks/task-1.md")).unwrap();
    assert!(archived_content.contains("Task v2 Updated"));
}

#[test]
fn test_archive_not_found() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("tasks")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["archive", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// ============================================================================
// Restore Tests
// ============================================================================

#[test]
fn test_archive_restore_moves_back_to_original() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("tasks")).unwrap();
    std::fs::create_dir_all(work_path.join("archive/tasks")).unwrap();

    // Create archived file
    std::fs::write(
        work_path.join("archive/tasks/task-1.md"),
        "---\ntype: task\nid: 1\nstatus: done\narchived_at: 2024-01-01T12:00:00Z\nindexed: false\n---\n\n# Task\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["archive", "restore", "task-1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Restored"));

    // Archive should be gone
    assert!(!work_path.join("archive/tasks/task-1.md").exists());

    // Original location should have file
    assert!(work_path.join("tasks/task-1.md").exists());
}

#[test]
fn test_archive_restore_removes_archived_at_and_reindexes() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("tasks")).unwrap();
    std::fs::create_dir_all(work_path.join("archive/tasks")).unwrap();

    std::fs::write(
        work_path.join("archive/tasks/task-1.md"),
        "---\ntype: task\nid: 1\nstatus: done\narchived_at: 2024-01-01T12:00:00Z\nindexed: false\n---\n\n# Task\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["archive", "restore", "task-1"])
        .assert()
        .success();

    let restored_content = std::fs::read_to_string(work_path.join("tasks/task-1.md")).unwrap();
    assert!(!restored_content.contains("archived_at:"));
    assert!(!restored_content.contains("indexed: false"));
}

#[test]
fn test_archive_restore_not_found_error() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("archive/tasks")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["archive", "restore", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

// ============================================================================
// List Tests
// ============================================================================

#[test]
fn test_archive_list_shows_archived_items() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("archive/tasks")).unwrap();
    std::fs::create_dir_all(work_path.join("archive/definitions")).unwrap();

    std::fs::write(
        work_path.join("archive/tasks/task-1.md"),
        "---\ntype: task\nid: 1\narchived_at: 2024-01-01T12:00:00Z\nindexed: false\n---\n",
    )
    .unwrap();
    std::fs::write(
        work_path.join("archive/definitions/term.md"),
        "---\ntype: definition\nid: term\narchived_at: 2024-01-02T12:00:00Z\nindexed: false\n---\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["archive", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("task-1"))
        .stdout(predicate::str::contains("term"));
}

#[test]
fn test_archive_list_empty_archive() {
    let tmp = tempdir().unwrap();

    let work_path = tmp.path().join("work");
    std::fs::create_dir_all(work_path.join("tasks")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["archive", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No archived items"));
}
