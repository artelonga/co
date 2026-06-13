//! Reference service — business rules for the references domain.
//!
//! CO-432: propagates the CO-390 service pattern from `entries` to `references`.
//! Pure rules only — no HTTP, no database, no async runtime. Infrastructure
//! concerns (file I/O, SQL, telemetry) stay in the repository and controller.

use std::path::PathBuf;

/// Reference business-rule service.
///
/// Zero-sized unit struct — all methods are associated functions (no state).
pub struct ReferenceService;

impl ReferenceService {
    /// A reference card entry is always `type: reference`, whatever the
    /// client sent. Extracted from `create_reference`.
    pub fn force_reference_type(frontmatter: &mut serde_json::Value) {
        if let Some(obj) = frontmatter.as_object_mut() {
            obj.insert(
                "type".to_string(),
                serde_json::Value::String("reference".to_string()),
            );
        }
    }

    /// Partial-update merge rule: a missing patch field keeps the existing
    /// value. Extracted from `update_reference`.
    pub fn merged_update(
        existing_frontmatter: &serde_json::Value,
        existing_body: &str,
        patch_frontmatter: Option<serde_json::Value>,
        patch_body: Option<String>,
    ) -> (serde_json::Value, String) {
        (
            patch_frontmatter.unwrap_or_else(|| existing_frontmatter.clone()),
            patch_body.unwrap_or_else(|| existing_body.to_string()),
        )
    }

    /// Derive a work_id slug from an entry path: take the filename stem.
    /// e.g. `refs/GNDicLex.md` → `GNDicLex`.
    pub fn work_id_from_path(path: &str) -> String {
        std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_string()
    }

    /// Seed-status rule: an edition that names a `file:` but whose blob could
    /// not be resolved on disk is forced back to `stub`, whatever the
    /// frontmatter claims. Extracted from the references_meta upsert.
    pub fn edition_seed_status(has_file: bool, has_blob: bool, raw_status: &str) -> String {
        if has_file && !has_blob {
            "stub".to_string()
        } else {
            raw_status.to_string()
        }
    }

    /// Where a card's bound blob is expected on disk, relative to the
    /// universe root: sibling of the card entry. Extracted from
    /// `broken_cards` / `compute_blob_sha256`.
    pub fn expected_blob_path(entry_path: &str, file: &str) -> Option<PathBuf> {
        let entry_dir = std::path::Path::new(entry_path).parent()?;
        Some(entry_dir.join(file))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn force_reference_type_overwrites_client_type() {
        let mut fm = json!({"type": "task", "title": "X"});
        ReferenceService::force_reference_type(&mut fm);
        assert_eq!(fm["type"], "reference");
        assert_eq!(fm["title"], "X");
    }

    #[test]
    fn merged_update_keeps_existing_when_patch_absent() {
        let existing = json!({"title": "old"});
        let (fm, body) = ReferenceService::merged_update(&existing, "old body", None, None);
        assert_eq!(fm, existing);
        assert_eq!(body, "old body");
    }

    #[test]
    fn merged_update_takes_patch_when_present() {
        let existing = json!({"title": "old"});
        let (fm, body) = ReferenceService::merged_update(
            &existing,
            "old body",
            Some(json!({"title": "new"})),
            Some("new body".into()),
        );
        assert_eq!(fm["title"], "new");
        assert_eq!(body, "new body");
    }

    #[test]
    fn work_id_is_filename_stem() {
        assert_eq!(
            ReferenceService::work_id_from_path("refs/GNDicLex.md"),
            "GNDicLex"
        );
        assert_eq!(ReferenceService::work_id_from_path("X.md"), "X");
    }

    #[test]
    fn unresolved_file_blob_forces_stub() {
        assert_eq!(
            ReferenceService::edition_seed_status(true, false, "reviewed"),
            "stub"
        );
        assert_eq!(
            ReferenceService::edition_seed_status(true, true, "reviewed"),
            "reviewed"
        );
        assert_eq!(
            ReferenceService::edition_seed_status(false, false, "seeded"),
            "seeded"
        );
    }

    #[test]
    fn expected_blob_path_is_sibling_of_entry() {
        assert_eq!(
            ReferenceService::expected_blob_path("refs/X.md", "x.pdf"),
            Some(PathBuf::from("refs/x.pdf"))
        );
    }
}
