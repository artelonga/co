//! Init command tests (US-1.2)

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_init_creates_space_directory() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    assert!(tmp.path().join("private").exists());
    assert!(tmp.path().join("private/README.md").exists());
}

#[test]
fn test_init_prevents_duplicate_space() {
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
fn test_init_creates_space_readme() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    let readme_path = tmp.path().join("private/README.md");
    assert!(readme_path.exists(), "README.md should exist");

    let readme = std::fs::read_to_string(&readme_path).unwrap();
    assert!(readme.contains("type: space"));
    assert!(readme.contains("id: private"));
    assert!(readme.contains("language: english"));
}

#[test]
fn test_list_shows_spaces_and_languages() {
    let tmp = tempdir().unwrap();

    // Create a language directory
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(&en_path).unwrap();
    std::fs::write(
        en_path.join("README.md"),
        "---\ntype: language\nid: en\n---\n# English\n",
    )
    .unwrap();

    // Create a space
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

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

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    let space_path = tmp.path().join("private");
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
