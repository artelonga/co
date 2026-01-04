//! Write command tests (Issue #39)
//!
//! Tests for `co write` - content generation using writer agents.

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

// ============================================================================
// Basic Command Tests
// ============================================================================

#[test]
fn test_write_requires_content_type() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["write", "--agent", "writer"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_write_requires_agent_flag() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["write", "user-story"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--agent"));
}

#[test]
fn test_write_validates_agent_exists() {
    let tmp = tempdir().unwrap();

    // Create space
    std::fs::create_dir_all(tmp.path().join("private")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args([
            "write",
            "user-story",
            "--agent",
            "nonexistent",
            "--in",
            "private",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_write_validates_space_exists() {
    let tmp = tempdir().unwrap();

    // Create agents directory with writer
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(
        tmp.path().join("agents/writer.md"),
        r#"---
type: agent
id: writer
backend: manual
---

# Writer Agent
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args([
            "write",
            "user-story",
            "--agent",
            "writer",
            "--in",
            "nonexistent",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn test_write_validates_content_type() {
    let tmp = tempdir().unwrap();

    // Create space and agents
    std::fs::create_dir_all(tmp.path().join("private")).unwrap();
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(
        tmp.path().join("agents/writer.md"),
        r#"---
type: agent
id: writer
backend: manual
---

# Writer Agent
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args([
            "write",
            "invalid-type",
            "--agent",
            "writer",
            "--in",
            "private",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown type"));
}

// ============================================================================
// Backend Tests
// ============================================================================

#[test]
fn test_write_manual_backend_creates_content() {
    let tmp = tempdir().unwrap();

    // Initialize space
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create agents directory with manual backend writer
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(
        tmp.path().join("agents/writer.md"),
        r#"---
type: agent
id: writer
backend: manual
---

# Writer Agent
"#,
    )
    .unwrap();

    // Simulate user input for manual mode
    // Format: name, context, as_a, i_need, so_that
    let input = "test-story\nContext for story\n\nproduct owner\nbetter search\nimprove UX\n";

    co_command()
        .current_dir(tmp.path())
        .args([
            "write",
            "user-story",
            "--agent",
            "writer",
            "--in",
            "private",
        ])
        .write_stdin(input)
        .assert()
        .success();

    // Verify file was created
    let file_path = tmp.path().join("private/user-stories/test-story.md");
    assert!(file_path.exists(), "User story file should be created");

    // Verify content
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("type: user-story"));
    assert!(content.contains("id: test-story"));
    assert!(content.contains("## As"));
    assert!(content.contains("product owner"));
}

#[test]
fn test_write_claude_backend_creates_skeleton() {
    let tmp = tempdir().unwrap();

    // Initialize space
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create agents directory with claude backend writer
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(
        tmp.path().join("agents/writer.md"),
        r#"---
type: agent
id: writer
backend: claude
model: claude-3-opus
capabilities: write, edit
---

# Writer Agent
"#,
    )
    .unwrap();

    // For claude backend, just provide the name
    let input = "claude-story\n";

    co_command()
        .current_dir(tmp.path())
        .args([
            "write",
            "user-story",
            "--agent",
            "writer",
            "--in",
            "private",
        ])
        .write_stdin(input)
        .assert()
        .success()
        .stdout(predicate::str::contains("skeleton"));

    // Verify file was created
    let file_path = tmp.path().join("private/user-stories/claude-story.md");
    assert!(file_path.exists(), "Skeleton file should be created");

    // Verify skeleton structure
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("type: user-story"));
    assert!(content.contains("## Prompt"));
    assert!(content.contains("## As"));
    assert!(content.contains("## I Need"));
    assert!(content.contains("## To"));
}

#[test]
fn test_write_ollama_backend_not_implemented() {
    let tmp = tempdir().unwrap();

    // Initialize space
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create agents directory with ollama backend writer
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(
        tmp.path().join("agents/local-writer.md"),
        r#"---
type: agent
id: local-writer
backend: ollama
model: llama2
---

# Local Writer Agent
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args([
            "write",
            "user-story",
            "--agent",
            "local-writer",
            "--in",
            "private",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not yet implemented"));
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_write_with_context_file() {
    let tmp = tempdir().unwrap();

    // Initialize space
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create agents directory
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(
        tmp.path().join("agents/writer.md"),
        r#"---
type: agent
id: writer
backend: claude
---

# Writer Agent
"#,
    )
    .unwrap();

    // Create context file
    std::fs::write(
        tmp.path().join("context.md"),
        "## Background\n\nThis is additional context for the story.\n",
    )
    .unwrap();

    let input = "context-story\n";

    co_command()
        .current_dir(tmp.path())
        .args([
            "write",
            "user-story",
            "--agent",
            "writer",
            "--in",
            "private",
            "--context",
            "context.md",
        ])
        .write_stdin(input)
        .assert()
        .success();

    // Verify context was included
    let file_path = tmp.path().join("private/user-stories/context-story.md");
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(
        content.contains("additional context"),
        "Context file content should be included"
    );
}

#[test]
fn test_write_task_with_manual_backend() {
    let tmp = tempdir().unwrap();

    // Initialize space
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create agents directory
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(
        tmp.path().join("agents/writer.md"),
        r#"---
type: agent
id: writer
backend: manual
---

# Writer Agent
"#,
    )
    .unwrap();

    // Simulate user input for task (GIVEN/WHEN/THEN)
    let input = "test-task\nTask context\n\nuser is logged in\nthey click button\naction happens\n";

    co_command()
        .current_dir(tmp.path())
        .args(["write", "task", "--agent", "writer", "--in", "private"])
        .write_stdin(input)
        .assert()
        .success();

    // Verify file was created
    let file_path = tmp.path().join("private/tasks/test-task.md");
    assert!(file_path.exists(), "Task file should be created");

    // Verify content
    let content = std::fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("type: task"));
    assert!(content.contains("## Given"));
    assert!(content.contains("user is logged in"));
}

#[test]
fn test_write_with_name_flag() {
    let tmp = tempdir().unwrap();

    // Initialize space
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create agents directory
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();
    std::fs::write(
        tmp.path().join("agents/writer.md"),
        r#"---
type: agent
id: writer
backend: claude
---

# Writer Agent
"#,
    )
    .unwrap();

    // Use --name flag instead of prompting
    co_command()
        .current_dir(tmp.path())
        .args([
            "write",
            "user-story",
            "--agent",
            "writer",
            "--in",
            "private",
            "--name",
            "named-story",
        ])
        .assert()
        .success();

    // Verify file was created with the specified name
    let file_path = tmp.path().join("private/user-stories/named-story.md");
    assert!(
        file_path.exists(),
        "File should be created with specified name"
    );
}
