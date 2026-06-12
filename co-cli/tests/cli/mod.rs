//! Shared test utilities for CLI integration tests

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;

/// Create a command for the CO CLI
pub fn co_command() -> Command {
    cargo_bin_cmd!("co")
}

// Test modules
pub mod analyze;
pub mod basic;
pub mod content;
pub mod create;
pub mod digest;
pub mod i18n;
pub mod init;
pub mod lead;
pub mod locate;
pub mod schema;
pub mod space;
pub mod store;
pub mod tools;
pub mod updates;
pub mod validate;
pub mod write;
