//! Entry service — business rules for the entries domain.
//!
//! CO-390 spike: proves Pattern 5 from `docs/spikes/library-manager-case-study.md`.
//!
//! ## What lives here
//!
//! Pure business rules, extracted from `entry_routes.rs`. They are:
//! - **Unit-testable** — no HTTP setup, no database, no async runtime.
//! - **Reusable** — the same rule can be called from HTTP handler, WebSocket, or CLI.
//! - **Explicitly named** — "anonymous quota" is no longer an unnamed if-block.
//!
//! Infrastructure concerns (file I/O, event bus, cache, telemetry) stay in the
//! controller layer — they require I/O and are better tested via integration tests.

use crate::domain::EntryDomain;
use crate::entry_index::EntryRow;
use crate::error::AppError;

/// Entry business-rule service.
///
/// Zero-sized unit struct — all methods are associated functions (no state).
/// The controller calls them directly; they return `Result<_, AppError>` so
/// callers can use `?` propagation without wrapping.
pub struct EntryService;

impl EntryService {
    // -----------------------------------------------------------------------
    // Write guards
    // -----------------------------------------------------------------------

    /// Reject the request if the universe's content count has reached the
    /// anonymous-user cap (100 entries).
    ///
    /// Extracted from `create_entry` in entry_routes.rs (CO-80 quota logic).
    /// The rule is: if the universe owner is anonymous (`owner_id` starts with
    /// `"anon-"`) AND `content_count >= 100`, the write is rejected.
    ///
    /// The controller checks whether the caller is authenticated BEFORE calling
    /// this; if auth exists, the tier-based quota path is taken instead.
    pub fn check_anon_quota(content_count: i64) -> Result<(), AppError> {
        if content_count >= 100 {
            Err(AppError::UsageLimitExceeded {
                current: content_count,
            })
        } else {
            Ok(())
        }
    }

    /// Reject writes on event-bus-backed universes.
    ///
    /// CO-383: universes backed by an event bus (e.g. Yggdrasil notes) are
    /// read-only from the CO API side.
    pub fn check_not_event_bus(
        source_kind: Option<&str>,
        source_url: Option<String>,
    ) -> Result<(), AppError> {
        if source_kind == Some("event-bus") {
            Err(AppError::ReadOnlyUniverse {
                source_kind: source_kind.unwrap_or_default().to_string(),
                source_url,
            })
        } else {
            Ok(())
        }
    }

    /// Validate frontmatter against the universe manifest's content-type schema.
    ///
    /// Returns `Ok(())` when:
    /// - `manifest` is `None` (no `_universe.yaml`).
    /// - The manifest has no schema for `entry_type`.
    /// - The payload passes all schema checks.
    ///
    /// Returns `Err(AppError::UnprocessableEntity)` with a field-path message
    /// when validation fails.
    ///
    /// Extracted from `validate_against_manifest()` in entry_routes.rs (CO-71).
    pub fn validate_entry_type(
        manifest: Option<&co::manifest::Manifest>,
        entry_type: &str,
        frontmatter: &serde_json::Value,
    ) -> Result<(), AppError> {
        // CO-368: Scrum entry types (pbi/sprint) carry a fixed frontmatter
        // shape contract enforced here, regardless of whether a manifest
        // declares them — so the sugar endpoints and the raw `/entries` POST
        // validate identically. No-op for every other type.
        crate::scrum::validate::validate_scrum_entry(entry_type, frontmatter)
            .map_err(AppError::UnprocessableEntity)?;

        let manifest = match manifest {
            Some(m) => m,
            None => return Ok(()),
        };
        let ct = match manifest
            .content_types
            .iter()
            .find(|ct| ct.name == entry_type)
        {
            Some(ct) => ct,
            None => return Ok(()),
        };
        co::payload::validate_payload(ct, frontmatter)
            .map_err(|e| AppError::UnprocessableEntity(format!("payload validation failed: {e}")))
    }

    // -----------------------------------------------------------------------
    // Visibility filters
    // -----------------------------------------------------------------------

    /// Apply the `public/` folder convention: anon visitors only see entries
    /// whose path starts with `public/`.
    ///
    /// Extracted from `filter_public_for_anon()` in entry_routes.rs (CO-268).
    ///
    /// Bypass conditions:
    /// - `caller_is_anon` is false (authenticated caller sees everything).
    /// - `universe_is_pub_sub` is true (CO-268 bypass).
    /// - The universe slug is not in the `public/` convention allowlist.
    pub fn apply_public_convention_filter(
        entries: Vec<EntryRow>,
        caller_is_anon: bool,
        slug: &str,
        universe_is_pub_sub: bool,
    ) -> Vec<EntryRow> {
        const PUBLIC_UNIVERSES: &[&str] = &["co"];
        if !PUBLIC_UNIVERSES.contains(&slug) || !caller_is_anon || universe_is_pub_sub {
            return entries;
        }
        entries
            .into_iter()
            .filter(|e| e.path.starts_with("public/") || e.path == "public")
            .collect()
    }

    /// Apply the published-only filter: anon callers cannot see unpublished entries.
    ///
    /// Extracted from `filter_published_for_anon()` in entry_routes.rs (CO-330).
    pub fn apply_published_filter(
        entries: Vec<EntryRow>,
        caller_is_anon: bool,
        anon_published_only: bool,
    ) -> Vec<EntryRow> {
        if !caller_is_anon || !anon_published_only {
            return entries;
        }
        entries
            .into_iter()
            .filter(|e| {
                e.frontmatter
                    .get("published")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Apply the review-pipeline filter: draft/reviewed entries are hidden from
    /// everyone except the universe owner and the original submitter.
    ///
    /// Extracted from `filter_review_status()` in entry_routes.rs (CO-354).
    /// Semantics are identical to the original; extracted so the rule is
    /// unit-testable without an HTTP server.
    pub fn apply_review_status_filter(
        entries: Vec<EntryRow>,
        is_owner: bool,
        viewer_key: &str,
    ) -> Vec<EntryRow> {
        if is_owner {
            return entries;
        }
        entries
            .into_iter()
            .filter(|e| Self::is_review_visible_for_row(&e.frontmatter, viewer_key))
            .collect()
    }

    fn is_review_visible_for_row(frontmatter: &serde_json::Value, viewer_key: &str) -> bool {
        let status = frontmatter
            .get("review_status")
            .and_then(|v| v.as_str())
            .unwrap_or("published");
        if status == "published" {
            return true;
        }
        frontmatter
            .get("submitted_by")
            .and_then(|v| v.as_str())
            .map(|s| s == viewer_key)
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Domain construction helpers
    // -----------------------------------------------------------------------

    /// Build a domain entity from a create request and the resulting core entry.
    ///
    /// Called by the controller after writing the file and indexing the entry.
    pub fn build_domain_from_create(
        universe_key: &str,
        path: &str,
        core_entry: &co::entry::Entry,
    ) -> EntryDomain {
        EntryDomain {
            path: path.to_string(),
            universe_key: universe_key.to_string(),
            entry_type: core_entry.entry_type.clone(),
            title: core_entry
                .frontmatter
                .get("title")
                .and_then(|v| v.as_str())
                .map(String::from),
            frontmatter: core_entry.frontmatter.clone(),
            body: core_entry.body.clone(),
            body_hash: core_entry.body_hash.clone(),
            created_at: Some(core_entry.stat.created.to_rfc3339()),
            updated_at: Some(core_entry.stat.modified.to_rfc3339()),
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests — the spike's testability payoff
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anon_quota_below_100_passes() {
        assert!(EntryService::check_anon_quota(0).is_ok());
        assert!(EntryService::check_anon_quota(99).is_ok());
    }

    #[test]
    fn anon_quota_at_100_fails() {
        let err = EntryService::check_anon_quota(100).unwrap_err();
        assert!(matches!(err, AppError::UsageLimitExceeded { current: 100 }));
    }

    #[test]
    fn anon_quota_above_100_fails() {
        let err = EntryService::check_anon_quota(150).unwrap_err();
        assert!(matches!(err, AppError::UsageLimitExceeded { .. }));
    }

    #[test]
    fn event_bus_universe_is_rejected() {
        let err = EntryService::check_not_event_bus(Some("event-bus"), None).unwrap_err();
        assert!(matches!(err, AppError::ReadOnlyUniverse { .. }));
    }

    #[test]
    fn non_event_bus_universe_is_accepted() {
        assert!(EntryService::check_not_event_bus(None, None).is_ok());
        assert!(EntryService::check_not_event_bus(Some("local"), None).is_ok());
    }

    #[test]
    fn published_filter_hides_unpublished_for_anon() {
        let entries = vec![
            make_row("public/1.md", serde_json::json!({"published": true})),
            make_row("public/2.md", serde_json::json!({"published": false})),
            make_row("public/3.md", serde_json::json!({})),
        ];
        let visible = EntryService::apply_published_filter(entries, true, true);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].path, "public/1.md");
    }

    #[test]
    fn published_filter_bypassed_for_auth_caller() {
        let entries = vec![make_row("1.md", serde_json::json!({"published": false}))];
        let visible = EntryService::apply_published_filter(entries.clone(), false, true);
        assert_eq!(visible.len(), 1);
    }

    #[test]
    fn public_convention_filters_non_public_for_anon() {
        let entries = vec![
            make_row("public/open.md", serde_json::json!({})),
            make_row("private/secret.md", serde_json::json!({})),
        ];
        let visible = EntryService::apply_public_convention_filter(entries, true, "co", false);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].path, "public/open.md");
    }

    #[test]
    fn public_convention_bypassed_for_pub_sub_universe() {
        let entries = vec![make_row("private/data.md", serde_json::json!({}))];
        let visible = EntryService::apply_public_convention_filter(entries, true, "co", true);
        assert_eq!(visible.len(), 1);
    }

    #[test]
    fn review_filter_hides_draft_from_non_owner() {
        let entries = vec![
            make_row_with_review("draft.md", "draft", "user-1"),
            make_row_with_review("pub.md", "published", ""),
        ];
        let visible = EntryService::apply_review_status_filter(entries, false, "user-2");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].path, "pub.md");
    }

    #[test]
    fn review_filter_shows_draft_to_submitter() {
        let entries = vec![make_row_with_review("draft.md", "draft", "user-1")];
        let visible = EntryService::apply_review_status_filter(entries, false, "user-1");
        assert_eq!(visible.len(), 1);
    }

    #[test]
    fn review_filter_shows_all_to_owner() {
        let entries = vec![
            make_row_with_review("draft.md", "draft", "user-99"),
            make_row_with_review("reviewed.md", "reviewed", "user-99"),
        ];
        let visible = EntryService::apply_review_status_filter(entries, true, "owner");
        assert_eq!(visible.len(), 2);
    }

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn make_row(path: &str, frontmatter: serde_json::Value) -> EntryRow {
        EntryRow {
            path: path.to_string(),
            universe_key: "test".to_string(),
            entry_type: "note".to_string(),
            title: None,
            frontmatter,
            body: String::new(),
            body_hash: String::new(),
            created_at: None,
            updated_at: None,
            _score: None,
        }
    }

    fn make_row_with_review(path: &str, review_status: &str, submitted_by: &str) -> EntryRow {
        let fm = serde_json::json!({
            "review_status": review_status,
            "submitted_by": submitted_by,
        });
        make_row(path, fm)
    }
}
