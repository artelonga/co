//! Tools command tests (Issue #40)
//!
//! Tests for `co tools run` - tool execution with arguments.

use super::co_command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

// ============================================================================
// co tools run - Basic Requirements
// ============================================================================

#[test]
fn test_tools_run_requires_name() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["tools", "run"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn test_tools_run_validates_tool_exists() {
    let tmp = tempdir().unwrap();

    // Create empty tools directory
    fs::create_dir_all(tmp.path().join("tools")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["tools", "run", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn test_tools_run_validates_command_field() {
    let tmp = tempdir().unwrap();

    // Create tool without command field
    fs::create_dir_all(tmp.path().join("tools")).unwrap();
    fs::write(
        tmp.path().join("tools/no-command.md"),
        r#"---
type: tool
id: no-command
category: test
---

# No Command Tool

This tool has no command field.
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["tools", "run", "no-command"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no executable command"));
}

// ============================================================================
// co tools run - Deterministic Tools
// ============================================================================

#[test]
fn test_tools_run_deterministic_executes() {
    let tmp = tempdir().unwrap();

    // Create deterministic tool that echoes "hello"
    fs::create_dir_all(tmp.path().join("tools")).unwrap();
    fs::write(
        tmp.path().join("tools/echo-test.md"),
        r#"---
type: tool
id: echo-test
category: test
tool_type: deterministic
command: echo hello
---

# Echo Test Tool

A simple echo test.
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["tools", "run", "echo-test"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello"));
}

#[test]
fn test_tools_run_with_args() {
    let tmp = tempdir().unwrap();

    // Create tool that echoes arguments
    fs::create_dir_all(tmp.path().join("tools")).unwrap();
    fs::write(
        tmp.path().join("tools/echo-args.md"),
        r#"---
type: tool
id: echo-args
category: test
tool_type: deterministic
command: echo
---

# Echo Args Tool

Echoes the provided arguments.
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["tools", "run", "echo-args", "foo", "bar", "baz"])
        .assert()
        .success()
        .stdout(predicate::str::contains("foo bar baz"));
}

// ============================================================================
// co tools run - Predictive Tools (Stub)
// ============================================================================

#[test]
fn test_tools_run_predictive_not_implemented() {
    let tmp = tempdir().unwrap();

    // Create predictive tool
    fs::create_dir_all(tmp.path().join("tools")).unwrap();
    fs::write(
        tmp.path().join("tools/whisper.md"),
        r#"---
type: tool
id: whisper
category: ml
tool_type: predictive
model: whisper
---

# Whisper Tool

Speech-to-text transcription (not yet implemented).
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["tools", "run", "whisper"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not yet implemented"));
}

// ============================================================================
// co tools run - Default Type Behavior
// ============================================================================

#[test]
fn test_tools_run_defaults_to_deterministic() {
    let tmp = tempdir().unwrap();

    // Create tool without explicit type (should default to deterministic)
    fs::create_dir_all(tmp.path().join("tools")).unwrap();
    fs::write(
        tmp.path().join("tools/default-type.md"),
        r#"---
type: tool
id: default-type
category: test
command: echo default works
---

# Default Type Tool

No tool_type specified, should default to deterministic.
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["tools", "run", "default-type"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default works"));
}

// ============================================================================
// co tools run - User Tools Override
// ============================================================================

#[test]
fn test_tools_run_user_tool_precedence() {
    let tmp = tempdir().unwrap();

    // Create system tool
    fs::create_dir_all(tmp.path().join("tools")).unwrap();
    fs::write(
        tmp.path().join("tools/override.md"),
        r#"---
type: tool
id: override
category: test
tool_type: deterministic
command: echo system
---

# System Override Tool
"#,
    )
    .unwrap();

    // Create user tool with same name (should take precedence)
    fs::create_dir_all(tmp.path().join("user/tools")).unwrap();
    fs::write(
        tmp.path().join("user/tools/override.md"),
        r#"---
type: tool
id: override
category: test
tool_type: deterministic
command: echo user
---

# User Override Tool
"#,
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["tools", "run", "override"])
        .assert()
        .success()
        .stdout(predicate::str::contains("user"));
}
