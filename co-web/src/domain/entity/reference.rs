//! CO domain entity — Reference card edition.
//!
//! Pure business type: no rusqlite, no axum, no HTTP deps.
//! Represents one edition row of a reference card (CO-156) — a `.md` entry
//! with `type: reference` that carries metadata about a bound binary asset
//! (sibling file) or external URL.
//!
//! CO-432: propagates the CO-390 layering template from `entries` to the
//! `references` entity.

/// One edition of a reference card.
///
/// A card (`entry_path`) may carry several editions (`editions:` array in
/// frontmatter); a single-edition card has `edition_id == "default"`.
///
/// Invariant: this struct has zero axum or rusqlite dependencies.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceDomain {
    /// Universe this card belongs to.
    pub universe_key: String,
    /// Vault-relative path of the card entry (e.g. `refs/GNDicLex.md`).
    pub entry_path: String,
    /// Edition identifier (`"default"` for single-edition cards).
    pub edition_id: String,
    /// Conceptual work identity — groups editions of the same work.
    pub work_id: String,
    /// Minimum source-chain layer (0 = phenomenon, 1 = transcription, …).
    pub primary_layer: Option<i64>,
    /// Sibling asset filename, if the edition is file-bound.
    pub file: Option<String>,
    /// SHA-256 of the bound blob, when resolvable.
    pub blob_sha256: Option<String>,
    /// External URL, if the edition is web-bound.
    pub url: Option<String>,
    /// Medium: pdf, image, video, audio, web, citation, …
    pub medium: String,
    pub mime: Option<String>,
    pub size_bytes: Option<i64>,
    pub language: Option<String>,
    /// Seed pipeline status (stub, seeded, reviewed, …).
    pub seed_status: String,
    /// ISO-8601 timestamp of the last index write.
    pub indexed_at: String,
    /// Title of the card entry (joined from the entries index).
    pub title: Option<String>,
}

impl ReferenceDomain {
    /// An edition is file-bound when it names a sibling asset file.
    pub fn is_file_bound(&self) -> bool {
        self.file.is_some()
    }

    /// An edition is a stub when it claims a file but no blob was resolved.
    pub fn is_stub(&self) -> bool {
        self.file.is_some() && self.blob_sha256.is_none()
    }
}
