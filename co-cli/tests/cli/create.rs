//! Create command tests (Issue #48)
//!
//! Tests for `co create` - interactive content creation with role selection.

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

// ============================================================================
// Basic Command Tests
// ============================================================================

#[test]
fn test_create_requires_content_type() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["create"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_create_requires_name() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["create", "user-story"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_create_validates_scope_exists() {
    let tmp = tempdir().unwrap();

    // Scope doesn't exist
    co_command()
        .current_dir(tmp.path())
        .args(["create", "user-story", "test-story", "--in", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn test_create_validates_content_type() {
    let tmp = tempdir().unwrap();

    // Create scope first
    std::fs::create_dir_all(tmp.path().join("private")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["create", "invalid-type", "test", "--in", "private"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown type"));
}

// ============================================================================
// Agent Role Tests (Non-interactive skeleton creation)
// ============================================================================

#[test]
fn test_create_agent_role_creates_user_story_skeleton() {
    let tmp = tempdir().unwrap();

    // Initialize scope
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create with piped input simulating agent role
    co_command()
        .current_dir(tmp.path())
        .args(["create", "user-story", "test-story", "--in", "private"])
        .write_stdin("agent\n")
        .assert()
        .success();

    // Verify file was created
    let file_path = tmp.path().join("private/user-stories/test-story.md");
    assert!(file_path.exists(), "User story file should be created");

    // Verify skeleton structure
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("type: user-story"));
    assert!(content.contains("id: test-story"));
    assert!(content.contains("## As"));
    assert!(content.contains("## I Need"));
    assert!(content.contains("## To"));
}

#[test]
fn test_create_agent_role_creates_task_skeleton() {
    let tmp = tempdir().unwrap();

    // Initialize scope
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create task with piped input simulating agent role
    co_command()
        .current_dir(tmp.path())
        .args(["create", "task", "test-task", "--in", "private"])
        .write_stdin("agent\n")
        .assert()
        .success();

    // Verify file was created
    let file_path = tmp.path().join("private/tasks/test-task.md");
    assert!(file_path.exists(), "Task file should be created");

    // Verify skeleton structure
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("type: task"));
    assert!(content.contains("id: test-task"));
    assert!(content.contains("## Given"));
    assert!(content.contains("## When"));
    assert!(content.contains("## Then"));
}

#[test]
fn test_create_task_with_story_link() {
    let tmp = tempdir().unwrap();

    // Initialize scope
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create task linked to a story
    co_command()
        .current_dir(tmp.path())
        .args([
            "create",
            "task",
            "linked-task",
            "--in",
            "private",
            "--story",
            "parent-story",
        ])
        .write_stdin("agent\n")
        .assert()
        .success();

    // Verify story link in frontmatter
    let file_path = tmp.path().join("private/tasks/linked-task.md");
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(
        content.contains("story:"),
        "Task should have story field in frontmatter"
    );
    assert!(
        content.contains("parent-story"),
        "Task should reference parent story"
    );
}

// ============================================================================
// User Role Tests (Piped input for testing)
// ============================================================================

#[test]
fn test_create_user_role_with_piped_input() {
    let tmp = tempdir().unwrap();

    // Initialize scope
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Simulate user role with piped structured input
    // Format: role, context, as_a, i_need, so_that
    let input = "user\nContext about the feature\n\nproduct owner\nbetter search\nimprove UX\n";

    co_command()
        .current_dir(tmp.path())
        .args(["create", "user-story", "search-story", "--in", "private"])
        .write_stdin(input)
        .assert()
        .success();

    // Verify file content
    let file_path = tmp.path().join("private/user-stories/search-story.md");
    let content = std::fs::read_to_string(&file_path).unwrap();

    assert!(content.contains("type: user-story"));
    assert!(content.contains("## Prompt"));
    assert!(content.contains("Context about the feature"));
    assert!(content.contains("## As"));
    assert!(content.contains("product owner"));
    assert!(content.contains("## I Need"));
    assert!(content.contains("better search"));
    assert!(content.contains("## To"));
    assert!(content.contains("improve UX"));
}

#[test]
fn test_create_task_user_role_with_piped_input() {
    let tmp = tempdir().unwrap();

    // Initialize scope
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Simulate user role with piped structured input for task
    // Format: role, context, given, when, then
    let input =
        "user\nContext for the task\n\nuser is logged in\nthey click search\nresults appear\n";

    co_command()
        .current_dir(tmp.path())
        .args(["create", "task", "search-task", "--in", "private"])
        .write_stdin(input)
        .assert()
        .success();

    // Verify file content
    let file_path = tmp.path().join("private/tasks/search-task.md");
    let content = std::fs::read_to_string(&file_path).unwrap();

    assert!(content.contains("type: task"));
    assert!(content.contains("## Prompt"));
    assert!(content.contains("Context for the task"));
    assert!(content.contains("## Given"));
    assert!(content.contains("user is logged in"));
    assert!(content.contains("## When"));
    assert!(content.contains("they click search"));
    assert!(content.contains("## Then"));
    assert!(content.contains("results appear"));
}
