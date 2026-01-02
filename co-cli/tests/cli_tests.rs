//! CLI integration tests

use std::process::Command;

fn co_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_co"))
}

#[test]
fn test_help() {
    let output = co_command()
        .arg("--help")
        .output()
        .expect("Failed to run co");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("co"));
}

#[test]
fn test_version() {
    let output = co_command()
        .arg("--version")
        .output()
        .expect("Failed to run co");
    assert!(output.status.success());
}

#[test]
fn test_status() {
    let output = co_command()
        .arg("status")
        .output()
        .expect("Failed to run co");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("CO Graph Status"));
}

#[test]
fn test_languages() {
    let output = co_command()
        .arg("languages")
        .output()
        .expect("Failed to run co");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("english"));
    assert!(stdout.contains("portuguese"));
    assert!(stdout.contains("math"));
}

#[test]
fn test_config_show() {
    let output = co_command()
        .args(["config", "show"])
        .output()
        .expect("Failed to run co");
    assert!(output.status.success());
}
