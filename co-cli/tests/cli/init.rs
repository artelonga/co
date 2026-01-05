//! Init command tests (US-1.2)

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_init_creates_namespace_directory() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created namespace"));

    // Directory should exist
    assert!(tmp.path().join("private").exists());
    assert!(tmp.path().join("private").is_dir());

    // No README.md created - user organizes files however they want
    assert!(!tmp.path().join("private/README.md").exists());
}

#[test]
fn test_init_prevents_duplicate_directory() {
    let tmp = tempdir().unwrap();

    std::fs::create_dir(tmp.path().join("private")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_list_shows_spaces_and_languages() {
    let tmp = tempdir().unwrap();

    // Create a language directory with README.md
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(&en_path).unwrap();
    std::fs::write(
        en_path.join("README.md"),
        "---\ntype: language\nid: en\n---\n# English\n",
    )
    .unwrap();

    // Create a space directory with README.md (manually, since init no longer does this)
    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(&private_path).unwrap();
    std::fs::write(
        private_path.join("README.md"),
        "---\ntype: space\nid: private\n---\n# Private\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("en"))
        .stdout(predicate::str::contains("language"))
        .stdout(predicate::str::contains("private"))
        .stdout(predicate::str::contains("space"));
}

#[test]
fn test_list_stats_shows_file_counts() {
    let tmp = tempdir().unwrap();

    // Create a space with README (list requires type: space in README)
    let space_path = tmp.path().join("private");
    std::fs::create_dir_all(&space_path).unwrap();
    std::fs::write(
        space_path.join("README.md"),
        "---\ntype: space\nid: private\n---\n",
    )
    .unwrap();
    std::fs::write(
        space_path.join("word1.md"),
        "---\ntype: definition\n---\nA definition.",
    )
    .unwrap();
    std::fs::write(
        space_path.join("word2.md"),
        "---\ntype: definition\n---\nAnother definition.",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["list", "--stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("private"))
        .stdout(predicate::str::contains("files"));
}

#[test]
fn test_init_does_not_modify_gitignore() {
    let tmp = tempdir().unwrap();

    // Create .gitignore first
    std::fs::write(tmp.path().join(".gitignore"), "target/\n").unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "myspace"])
        .assert()
        .success();

    // Verify .gitignore was NOT modified (simplified init)
    let gitignore = std::fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
    assert!(!gitignore.contains("myspace/"));
    assert_eq!(gitignore, "target/\n");
}

#[test]
fn test_init_warns_if_space_directory_not_gitignored() {
    let tmp = tempdir().unwrap();

    // Create a directory that looks like an existing space but isn't gitignored
    let existing_space = tmp.path().join("existing");
    std::fs::create_dir(&existing_space).unwrap();
    std::fs::write(
        existing_space.join("README.md"),
        "---\ntype: space\nid: existing\n---\n",
    )
    .unwrap();

    // Create .gitignore without the space
    std::fs::write(tmp.path().join(".gitignore"), "target/\n").unwrap();

    // Run co init check (new subcommand) to warn about unprotected spaces
    co_command()
        .current_dir(tmp.path())
        .args(["init", "--check"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("existing").and(predicate::str::contains("not gitignored")),
        );
}
