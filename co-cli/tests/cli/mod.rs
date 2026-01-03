//! Shared test utilities for CLI integration tests

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;

/// Create a command for the CO CLI
pub fn co_command() -> Command {
    cargo_bin_cmd!("co")
}

// Test modules
pub mod basic;
pub mod content;
pub mod context;
pub mod i18n;
pub mod lead;
pub mod locate;
pub mod schema;
pub mod validate;
