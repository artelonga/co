//! Basic CLI tests (help, version, status)

use super::co_command;
use predicates::prelude::*;

#[test]
fn test_help() {
    co_command()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("co"));
}

#[test]
fn test_version() {
    co_command().arg("--version").assert().success();
}

#[test]
fn test_status() {
    co_command()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("CO Graph Status"));
}

#[test]
fn test_config_show() {
    co_command().args(["config", "show"]).assert().success();
}

#[test]
fn test_status_shows_space_context_at_repo_root() {
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();

    // Create .git to simulate repo root
    std::fs::create_dir(tmp.path().join(".git")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Location:").and(predicate::str::contains("repo root")));
}

#[test]
fn test_status_shows_space_context_in_space() {
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();

    // Create .git for repo root
    std::fs::create_dir(tmp.path().join(".git")).unwrap();

    // Create a space
    let space_path = tmp.path().join("myspace");
    std::fs::create_dir(&space_path).unwrap();
    std::fs::write(
        space_path.join("README.md"),
        "---\ntype: space\nid: myspace\n---\n",
    )
    .unwrap();

    co_command()
        .current_dir(&space_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("Location:").and(predicate::str::contains("myspace")));
}
