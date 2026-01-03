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
