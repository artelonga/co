//! Interactive lead command tests (US-8.1)

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_lead_command_exists() {
    co_command()
        .args(["lead", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Interactive"));
}

#[test]
fn test_lead_exits_on_quit() {
    co_command()
        .arg("lead")
        .write_stdin("quit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Goodbye"));
}

#[test]
fn test_lead_exits_on_exit() {
    co_command()
        .arg("lead")
        .write_stdin("exit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("Goodbye"));
}

#[test]
fn test_lead_shows_prompt_with_scope() {
    co_command()
        .arg("lead")
        .write_stdin("exit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("co ["));
}

#[test]
fn test_lead_use_command_switches_scope() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    co_command()
        .current_dir(tmp.path())
        .arg("lead")
        .write_stdin("use private\nexit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("co [private]"));
}

#[test]
fn test_lead_help_shows_commands() {
    co_command()
        .arg("lead")
        .write_stdin("help\nexit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("use"))
        .stdout(predicate::str::contains("exit"));
}

#[test]
fn test_lead_status_command() {
    co_command()
        .arg("lead")
        .write_stdin("status\nexit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("CO Graph Status"));
}

#[test]
fn test_lead_locate_command() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .arg("lead")
        .write_stdin("locate status:todo\nexit\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("task1"));
}
