//! Schema and extensible content type tests (Issue #35)

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

// ============================================================================
// AC-1: User can create `<type>/schema.yaml` to define new content type
// ============================================================================

#[test]
fn test_custom_type_discovered_from_schema() {
    let tmp = tempdir().unwrap();

    // Create a custom "note" type with schema.yaml
    let notes_dir = tmp.path().join("notes");
    std::fs::create_dir_all(&notes_dir).unwrap();
    std::fs::write(
        notes_dir.join("schema.yaml"),
        r#"name: notes
version: 1
properties:
  status:
    kind: text
    required: true
  tags:
    kind: list
"#,
    )
    .unwrap();

    // co schema list should show the custom type
    co_command()
        .current_dir(tmp.path())
        .args(["schema", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("notes"));
}

// ============================================================================
// AC-2: `co new <type> <name>` creates content from any registered type
// ============================================================================

#[test]
fn test_new_creates_custom_type_content() {
    let tmp = tempdir().unwrap();

    // Create custom type directory with schema that defines the "note" content type
    let notes_dir = tmp.path().join("notes");
    std::fs::create_dir_all(&notes_dir).unwrap();
    std::fs::write(
        notes_dir.join("schema.yaml"),
        r#"name: notes
version: 1
content_types:
  - note
properties:
  status:
    kind: text
"#,
    )
    .unwrap();

    // Create scope
    let scope_dir = tmp.path().join("private");
    std::fs::create_dir_all(&scope_dir).unwrap();

    // Should be able to create note type
    co_command()
        .current_dir(tmp.path())
        .args(["new", "note", "my-note", "--in", "private"])
        .assert()
        .success();

    let note_path = tmp.path().join("private/notes/my-note.md");
    assert!(note_path.exists(), "Note file should be created");
}

#[test]
fn test_new_rejects_unknown_type() {
    let tmp = tempdir().unwrap();

    // Create scope but no custom type
    let scope_dir = tmp.path().join("private");
    std::fs::create_dir_all(&scope_dir).unwrap();

    // Should fail for unregistered type
    co_command()
        .current_dir(tmp.path())
        .args(["new", "unknown-type", "foo", "--in", "private"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("Unknown type").or(predicate::str::contains("unknown type")),
        );
}

// ============================================================================
// AC-3: Built-in types: epic, user-story, task, use-case
// ============================================================================

#[test]
fn test_builtin_type_epic_recognized() {
    let tmp = tempdir().unwrap();

    // Create work directory with schema (built-in types should be there)
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).unwrap();
    std::fs::write(
        work_dir.join("schema.yaml"),
        r#"name: work
version: 1
content_types:
  - epic
  - user-story
  - task
  - use-case
properties:
  status:
    kind: text
    required: true
"#,
    )
    .unwrap();

    // Create en scope
    let en_dir = tmp.path().join("en");
    std::fs::create_dir_all(&en_dir).unwrap();

    // Should create epic without error
    co_command()
        .current_dir(tmp.path())
        .args(["new", "epic", "my-epic", "--in", "en"])
        .assert()
        .success();
}

#[test]
fn test_builtin_types_in_schema_list() {
    let tmp = tempdir().unwrap();

    // Create work directory with built-in types
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).unwrap();
    std::fs::write(
        work_dir.join("schema.yaml"),
        r#"name: work
version: 1
content_types:
  - epic
  - user-story
  - task
  - use-case
properties:
  status:
    kind: text
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["schema", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("work"));
}

// ============================================================================
// AC-4: Validation respects custom schemas
// ============================================================================

#[test]
fn test_validation_accepts_custom_type_with_schema() {
    let tmp = tempdir().unwrap();

    // Create custom "note" type
    let notes_dir = tmp.path().join("notes");
    std::fs::create_dir_all(&notes_dir).unwrap();
    std::fs::write(
        notes_dir.join("schema.yaml"),
        r#"name: notes
version: 1
content_types:
  - note
properties:
  status:
    kind: text
"#,
    )
    .unwrap();

    // Create en directory and a note file
    let en_dir = tmp.path().join("en");
    std::fs::create_dir_all(en_dir.join("notes")).unwrap();
    std::fs::write(
        en_dir.join("notes/my-note.md"),
        r#"---
type: note
id: my-note
language: english
status: draft
---

# My Note

Just some thoughts.
"#,
    )
    .unwrap();

    // Validation should NOT warn about unknown type
    co_command()
        .current_dir(tmp.path())
        .args(["validate", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Unknown type: note").not());
}

#[test]
fn test_validation_still_warns_for_truly_unknown_types() {
    let tmp = tempdir().unwrap();

    // Create en directory with content that has unknown type
    let en_dir = tmp.path().join("en");
    std::fs::create_dir_all(en_dir.join("misc")).unwrap();
    std::fs::write(
        en_dir.join("misc/random.md"),
        r#"---
type: completely-random-type
id: random
language: english
---

# Random

Content.
"#,
    )
    .unwrap();

    // Should still warn about truly unknown type
    co_command()
        .current_dir(tmp.path())
        .args(["validate", "all"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Unknown type: completely-random-type",
        ));
}

// ============================================================================
// AC-5: `co schema list` shows all available types
// ============================================================================

#[test]
fn test_schema_list_shows_all_types() {
    let tmp = tempdir().unwrap();

    // Create multiple custom type directories
    for (name, props) in [
        ("articles", "title"),
        ("recipes", "ingredients"),
        ("projects", "deadline"),
    ] {
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("schema.yaml"),
            format!(
                r#"name: {name}
version: 1
properties:
  {props}:
    kind: text
"#
            ),
        )
        .unwrap();
    }

    co_command()
        .current_dir(tmp.path())
        .args(["schema", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("articles"))
        .stdout(predicate::str::contains("recipes"))
        .stdout(predicate::str::contains("projects"));
}
