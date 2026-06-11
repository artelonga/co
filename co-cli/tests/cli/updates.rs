//! CO-404: `co updates` — release notes in the CLI.

use super::co_command;

/// Default: shows the most recent release section from the embedded CHANGELOG.
#[test]
fn test_updates_shows_latest_release() {
    let assert = co_command().arg("updates").assert().success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    // The latest release header (version + theme) must be present.
    assert!(
        out.contains("3.1.0"),
        "expected latest version in output, got:\n{out}"
    );
    assert!(
        out.contains("delivery pipeline"),
        "expected release theme in output, got:\n{out}"
    );
    // Task-level entries from the section body.
    assert!(
        out.contains("CO-395") || out.contains("CO-392") || out.contains("CO-398"),
        "expected at least one CO-N entry, got:\n{out}"
    );
    // Older releases must NOT appear by default.
    assert!(
        !out.contains("3.0.0]"),
        "default output must show only the latest release, got:\n{out}"
    );
}

/// -n 2 shows the two most recent releases.
#[test]
fn test_updates_n_flag_shows_multiple_releases() {
    let assert = co_command().args(["updates", "-n", "2"]).assert().success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(out.contains("3.1.0"), "expected 3.1.0, got:\n{out}");
    assert!(out.contains("3.0.0"), "expected 3.0.0, got:\n{out}");
}

/// --all lists every release header.
#[test]
fn test_updates_all_lists_release_history() {
    let assert = co_command().args(["updates", "--all"]).assert().success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert!(out.contains("3.1.0"), "expected 3.1.0, got:\n{out}");
    assert!(
        out.contains("2.43.0"),
        "expected older release in --all history, got:\n{out}"
    );
}
