//! Content CRUD tests (US-2.x)

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

// ============================================================================
// US-2.1: Create Content Tests
// ============================================================================

#[test]
fn test_new_creates_file_in_space() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    co_command()
        .current_dir(tmp.path())
        .args(["new", "task", "my-task", "--in", "private"])
        .assert()
        .success();

    let task_path = tmp.path().join("private/tasks/my-task.md");
    assert!(task_path.exists());
}

#[test]
fn test_new_auto_populates_frontmatter() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    co_command()
        .current_dir(tmp.path())
        .args(["new", "task", "my-task", "--in", "private"])
        .assert()
        .success();

    let task_path = tmp.path().join("private/tasks/my-task.md");
    let content = std::fs::read_to_string(&task_path).unwrap();

    assert!(content.contains("schema_version: 2"));
    assert!(content.contains("language: en"));
    assert!(content.contains("space: private"));
    assert!(content.contains("type: task"));
    assert!(content.contains("status: todo"));
}

#[test]
fn test_new_defaults_to_current_directory() {
    let tmp = tempdir().unwrap();

    // No need to create an 'en' directory - should work in current dir
    co_command()
        .current_dir(tmp.path())
        .args(["new", "task", "foo"])
        .assert()
        .success();

    // File should be created in current directory (tasks/)
    let task_path = tmp.path().join("tasks/foo.md");
    assert!(task_path.exists());

    // Space name in frontmatter should be the temp dir's name (not ".")
    let content = std::fs::read_to_string(&task_path).unwrap();
    assert!(content.contains("type: task"));
    assert!(content.contains("id: foo"));
}

// ============================================================================
// US-2.2: Read Content Tests
// ============================================================================

#[test]
fn test_show_displays_content() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/todo.md"),
        "---\ntype: task\nid: todo\nstatus: todo\n---\n\n# My Task\n\nTask content here.\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["show", "todo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("My Task"))
        .stdout(predicate::str::contains("Task content here"));
}

#[test]
fn test_show_meta_only() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/todo.md"),
        "---\ntype: task\nid: todo\nstatus: todo\n---\n\n# My Task\n\nTask content here.\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["show", "todo", "--meta"])
        .assert()
        .success()
        .stdout(predicate::str::contains("type: task"))
        .stdout(predicate::str::contains("status: todo"))
        .stdout(predicate::str::contains("My Task").not());
}

// ============================================================================
// US-2.3: Update Content Tests
// ============================================================================

#[test]
fn test_update_changes_status() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/todo.md"),
        "---\ntype: task\nid: todo\nstatus: todo\n---\n\n# My Task\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["update", "todo", "--status", "done"])
        .assert()
        .success();

    let content = std::fs::read_to_string(en_path.join("tasks/todo.md")).unwrap();
    assert!(content.contains("status: done"));
    assert!(!content.contains("status: todo"));
}

// ============================================================================
// US-2.4: Delete Content Tests
// ============================================================================

#[test]
fn test_delete_removes_file() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    let task_path = en_path.join("tasks/old.md");
    std::fs::write(
        &task_path,
        "---\ntype: task\nid: old\nstatus: todo\n---\n\n# Old Task\n",
    )
    .unwrap();

    assert!(task_path.exists());

    co_command()
        .current_dir(tmp.path())
        .args(["delete", "old", "--confirm"])
        .assert()
        .success();

    assert!(!task_path.exists());
}
