//! Validation tests (US-5.1)

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_validate_all_valid_files() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    // Task type requires Given/When/Then sections (EPIC 13)
    std::fs::write(
        en_path.join("tasks/task1.md"),
        r#"---
type: task
id: task1
language: english
---

# Task 1

## Given
a valid task

## When
validation runs

## Then
no issues found
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["validate", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("no issues found"));
}

#[test]
fn test_validate_missing_language_error() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/bad-task.md"),
        "---\ntype: task\nid: bad-task\n---\n\n# Bad Task\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["validate", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Missing required field: language"));
}

#[test]
fn test_validate_unknown_language_error() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nlanguage: nonexistent\n---\n\n# Task 1\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["validate", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unknown language: nonexistent"));
}

#[test]
fn test_validate_unknown_scope_error() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nlanguage: english\nscope: nonexistent\n---\n\n# Task 1\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["validate", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unknown scope: nonexistent"));
}

#[test]
fn test_validate_broken_link_warning() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nlanguage: english\n---\n\nSee [[nonexistent-task]] for details.\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["validate", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Broken link: nonexistent-task"));
}

#[test]
fn test_validate_item_specific() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    // Task type requires Given/When/Then sections (EPIC 13)
    std::fs::write(
        en_path.join("tasks/my-task.md"),
        r#"---
type: task
id: my-task
language: english
---

# My Task

## Given
a specific task

## When
item validation runs

## Then
task is valid
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["validate", "item", "my-task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("is valid"));
}

#[test]
fn test_validate_item_not_found() {
    let tmp = tempdir().unwrap();

    std::fs::create_dir_all(tmp.path().join("en")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["validate", "item", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_validate_unknown_type_warning() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("misc")).unwrap();
    std::fs::write(
        en_path.join("misc/custom.md"),
        "---\ntype: custom-type\nid: custom\nlanguage: english\n---\n\n# Custom Content\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["validate", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unknown type: custom-type"));
}
