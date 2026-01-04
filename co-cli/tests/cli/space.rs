//! Space and repo command tests

use super::co_command;
use predicates::prelude::*;

#[test]
fn test_space_list_help() {
    co_command()
        .args(["space", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("List all registered spaces"))
        .stdout(predicate::str::contains("Show current space details"));
}

#[test]
fn test_space_list_empty() {
    // This test may show existing spaces if any are registered
    co_command().args(["space", "list"]).assert().success();
}

#[test]
fn test_space_current_outside_space() {
    // Run from a temp directory to ensure we're not in a registered space
    co_command()
        .current_dir(std::env::temp_dir())
        .args(["space", "current"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Not in a CO workspace"))
        .stdout(predicate::str::contains("co repo add"))
        .stdout(predicate::str::contains("co repo switch"));
}

#[test]
fn test_repo_add_help() {
    co_command()
        .args(["repo", "add", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--alias"))
        .stdout(predicate::str::contains("--ssh-host"));
}

#[test]
fn test_repo_list_help() {
    co_command()
        .args(["repo", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("add"))
        .stdout(predicate::str::contains("remove"))
        .stdout(predicate::str::contains("switch"));
}

#[test]
fn test_repo_switch_help() {
    co_command()
        .args(["repo", "switch", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Switch active workspace context"));
}
