//! Analyze command tests (Issue #41)

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_analyze_item_not_found() {
    let tmp = tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("en")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["analyze", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_analyze_shows_title_and_type() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("user-stories")).unwrap();
    std::fs::write(
        en_path.join("user-stories/my-story.md"),
        r#"---
type: user-story
id: my-story
language: english
status: draft
---

# My User Story

## As
a developer

## I Need
to analyze content

## To
identify gaps
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["analyze", "my-story"])
        .assert()
        .success()
        .stdout(predicate::str::contains("User-Story: My User Story"))
        .stdout(predicate::str::contains("Status: draft"));
}

#[test]
fn test_analyze_reports_missing_sections() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("user-stories")).unwrap();
    std::fs::write(
        en_path.join("user-stories/incomplete-story.md"),
        r#"---
type: user-story
id: incomplete-story
language: english
---

# Incomplete Story

## As
a user
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["analyze", "incomplete-story"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Missing required section: ## I Need",
        ))
        .stdout(predicate::str::contains("Missing required section: ## To"));
}

#[test]
fn test_analyze_checks_task_sections() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/incomplete-task.md"),
        r#"---
type: task
id: incomplete-task
language: english
---

# Incomplete Task

## Given
some context
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["analyze", "incomplete-task"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Missing required section: ## When",
        ))
        .stdout(predicate::str::contains(
            "Missing required section: ## Then",
        ));
}

#[test]
fn test_analyze_checks_linked_items() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/linked-task.md"),
        r#"---
type: task
id: linked-task
language: english
---

# Linked Task

See [[nonexistent-ref]] for details.

## Given
context

## When
action

## Then
outcome
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["analyze", "linked-task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Broken link: nonexistent-ref"));
}

#[test]
fn test_analyze_generates_suggestions() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("user-stories")).unwrap();
    std::fs::write(
        en_path.join("user-stories/no-status.md"),
        r#"---
type: user-story
id: no-status
language: english
---

# Story Without Status

## As
a user

## I Need
feature

## To
get value
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["analyze", "no-status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Suggestions:"))
        .stdout(predicate::str::contains("Set status field"));
}

#[test]
fn test_analyze_generates_questions() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("user-stories")).unwrap();
    std::fs::write(
        en_path.join("user-stories/incomplete.md"),
        r#"---
type: user-story
id: incomplete
language: english
---

# Incomplete Story

## As
a user
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["analyze", "incomplete"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Interview questions:"))
        .stdout(predicate::str::contains(
            "What specific functionality is needed?",
        ));
}

#[test]
fn test_analyze_verbose_output() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/verbose-task.md"),
        r#"---
type: task
id: verbose-task
language: english
status: in-progress
---

# Verbose Task

## Given
context

## When
action

## Then
outcome
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["analyze", "verbose-task", "-v"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Task: Verbose Task"))
        .stdout(predicate::str::contains("Has clear title"));
}

#[test]
fn test_analyze_pass_indicators() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/complete-task.md"),
        r#"---
type: task
id: complete-task
language: english
status: done
---

# Complete Task

## Given
complete context

## When
complete action

## Then
complete outcome
"#,
    )
    .unwrap();

    // Should show pass indicators for complete content
    co_command()
        .current_dir(tmp.path())
        .args(["analyze", "complete-task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Has clear title"))
        .stdout(predicate::str::contains("Has status field"))
        .stdout(predicate::str::contains("Has all required sections"));
}
