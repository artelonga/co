//! Request DTO — query parameters for `GET /references`.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ListRefsQuery {
    pub medium: Option<String>,
    pub seed_status: Option<String>,
    /// Filter by conceptual work identity (returns all editions of that work).
    pub work_id: Option<String>,
    /// Filter by minimum source-chain layer (0=phenomenon, 1=transcription, …).
    pub primary_layer: Option<i64>,
    /// Full-text search across title, body, transcription.
    pub q: Option<String>,
}
