//! Response DTO — a reference card whose `file:` doesn't resolve on disk.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct BrokenCard {
    pub entry_path: String,
    pub file: String,
    pub expected_path: String,
}
