//! CO-404: `co updates` — release notes in the CLI.

use super::co_command;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn release_header_count(out: &str) -> usize {
    out.lines()
        .filter(|l| l.trim_start().starts_with('[') && l.contains("] — "))
        .count()
}

/// Default: exactly one release section — and it matches the binary's own
/// version (CHANGELOG and Cargo.toml are bumped by the same release commit).
#[test]
fn test_updates_shows_latest_release() {
    let assert = co_command().arg("updates").assert().success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(
        out.contains(VERSION),
        "latest note must match binary version {VERSION}, got:\n{out}"
    );
    assert_eq!(
        release_header_count(&out),
        1,
        "default output must show exactly one release, got:\n{out}"
    );
}

/// -n 2 shows exactly two release sections, newest first.
#[test]
fn test_updates_n_flag_shows_multiple_releases() {
    let assert = co_command().args(["updates", "-n", "2"]).assert().success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(out.contains(VERSION), "expected {VERSION}, got:\n{out}");
    assert_eq!(release_header_count(&out), 2, "expected 2 releases:\n{out}");
}

/// --all lists deep release history.
#[test]
fn test_updates_all_lists_release_history() {
    let assert = co_command().args(["updates", "--all"]).assert().success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(out.contains(VERSION), "expected {VERSION}, got:\n{out}");
    assert!(
        out.contains("0.1.0"),
        "expected origin release in --all history, got:\n{out}"
    );
}
