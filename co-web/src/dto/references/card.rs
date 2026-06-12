//! Response DTO — one reference card edition, joined with the entry title.

use serde::{Deserialize, Serialize};

/// A row from `references_meta` joined with the entry title.
///
/// Wire-identical to the pre-CO-432 `reference_routes::ReferenceCard`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceCard {
    pub universe_key: String,
    pub entry_path: String,
    pub edition_id: String,
    pub work_id: String,
    pub primary_layer: Option<i64>,
    pub file: Option<String>,
    pub blob_sha256: Option<String>,
    pub url: Option<String>,
    pub medium: String,
    pub mime: Option<String>,
    pub size_bytes: Option<i64>,
    pub language: Option<String>,
    pub seed_status: String,
    pub indexed_at: String,
    pub title: Option<String>,
}
