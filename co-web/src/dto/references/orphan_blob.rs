//! Response DTO — an asset with no corresponding reference card.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct OrphanBlob {
    pub sha256: String,
    pub mime: String,
    pub size_bytes: i64,
    pub filename: Option<String>,
}
