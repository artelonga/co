//! CO-439: Allowlist-on-serve — serve only published/indexed content.
//!
//! The surfaces server serves **only** content present in a universe's
//! published index (the per-universe `entries` table) — never a raw file that
//! merely happens to exist on disk. A path that is not indexed resolves to
//! **404**, not 200; for an anonymous caller on a `published`-gated universe an
//! indexed-but-unpublished entry is likewise not servable.
//!
//! `.dockerignore` is a *denylist* and fails open: anything it does not know
//! about leaks. This allowlist is the actual security boundary. (Post-mortem:
//! the `ArteLonga/yuri` `thrive market.md` draft leak, 2026-06 — a private
//! draft went public because it sat in a served directory, was missing from the
//! `.dockerignore`, and existed on disk at deploy time. The structural fix is
//! to serve from the index, not from disk.)

use crate::entry_index::EntryRow;
use crate::server::AppState;

/// Candidate index paths the SPA might resolve `subpath` to.
///
/// Mirrors `maybeOpenEntryFromUrl` in `app.js` plus the CO-264 well-known
/// aliases (`CHANGELOG`/`README`/`LICENSE`) and folder-level `index.md` for
/// trailing-slash paths.
pub(crate) fn candidate_paths(subpath: &str) -> Vec<String> {
    let mut candidates = vec![
        format!("{subpath}.md"),
        subpath.to_string(),
        format!("content/{subpath}.md"),
        format!("content/{subpath}"),
    ];

    // Folder-level index.md for trailing-slash paths (e.g. `public/`).
    if subpath.ends_with('/') {
        candidates.push(format!("{subpath}index.md"));
        candidates.push(format!("{subpath}index"));
    }

    // Well-known file aliases (case-insensitive subpath match).
    match subpath.to_lowercase().as_str() {
        "changelog" => {
            candidates.push("CHANGELOG.md".to_string());
            candidates.push("changelog.md".to_string());
        }
        "readme" => {
            candidates.push("README.md".to_string());
            candidates.push("readme.md".to_string());
        }
        "license" => {
            candidates.push("LICENSE.md".to_string());
            candidates.push("LICENSE".to_string());
        }
        _ => {}
    }

    candidates
}

/// Look up the indexed entry for `subpath` in `universe_slug`, if any.
///
/// Returns `None` when the universe does not exist, the per-universe connection
/// cannot be acquired, or none of the [`candidate_paths`] are present in the
/// index — i.e. the path is **not on the allowlist**.
fn lookup_indexed_entry(state: &AppState, universe_slug: &str, subpath: &str) -> Option<EntryRow> {
    let uc = {
        let storage = state.core.storage.lock();
        storage.get_universe(universe_slug)?;
        storage.universe_conn(universe_slug)
    };
    let repo = crate::repository::SqliteEntryRepository::new(uc);
    candidate_paths(subpath)
        .iter()
        .find_map(|p| repo.get(universe_slug, p).ok().flatten())
}

/// Pure published-visibility gate: is `frontmatter` servable to a caller?
///
/// Authenticated callers and non-gated universes always pass. On a
/// `anon_published_only` universe an anonymous caller only sees entries
/// explicitly marked `published: true` (CO-324/CO-330). Mirrors the API-layer
/// gate in `entries/routes.rs::is_published_for_anon` so the surfaces server
/// and the content API agree.
pub(crate) fn is_published_for_anon(
    frontmatter: &serde_json::Value,
    is_anon: bool,
    anon_published_only: bool,
) -> bool {
    if !is_anon || !anon_published_only {
        return true;
    }
    frontmatter
        .get("published")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// CO-439: is `subpath` servable from `universe_slug` to this caller?
///
/// `true` only when an index entry exists for one of the [`candidate_paths`]
/// **and** that entry is visible to the caller (the published gate for anon).
/// A draft sitting on disk but absent from the index is never servable.
pub(crate) fn is_servable(
    state: &AppState,
    universe_slug: &str,
    subpath: &str,
    is_anon: bool,
) -> bool {
    let Some(entry) = lookup_indexed_entry(state, universe_slug, subpath) else {
        return false;
    };
    let anon_published_only = {
        let storage = state.core.storage.lock();
        storage
            .get_universe(universe_slug)
            .map(|u| u.anon_published_only)
            .unwrap_or(false)
    };
    is_published_for_anon(&entry.frontmatter, is_anon, anon_published_only)
}

/// CO-439 audit: the "servable but not published" leak surface.
///
/// Given the set of indexed entry paths for a universe and the relative paths
/// of content files found on disk under its served root, return the disk paths
/// that are **not** in the index. What remains is exactly what the surfaces
/// server would have served from disk but the allowlist now refuses — i.e. the
/// leak surface a `.dockerignore` denylist could have missed. Bookkeeping files
/// (SQLite db/-wal/-shm, manifest) are excluded by the caller before this point.
pub fn unindexed_disk_paths(
    indexed: &std::collections::HashSet<String>,
    disk_rel_paths: &[String],
) -> Vec<String> {
    let mut out: Vec<String> = disk_rel_paths
        .iter()
        .filter(|p| !indexed.contains(*p))
        .cloned()
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn candidate_paths_covers_md_and_content_variants() {
        let c = candidate_paths("notes/foo");
        assert!(c.contains(&"notes/foo.md".to_string()));
        assert!(c.contains(&"notes/foo".to_string()));
        assert!(c.contains(&"content/notes/foo.md".to_string()));
        assert!(c.contains(&"content/notes/foo".to_string()));
    }

    #[test]
    fn candidate_paths_adds_folder_index_for_trailing_slash() {
        let c = candidate_paths("public/");
        assert!(c.contains(&"public/index.md".to_string()));
        assert!(c.contains(&"public/index".to_string()));
    }

    #[test]
    fn candidate_paths_adds_wellknown_aliases() {
        assert!(candidate_paths("changelog").contains(&"CHANGELOG.md".to_string()));
        assert!(candidate_paths("README").contains(&"README.md".to_string()));
        assert!(candidate_paths("license").contains(&"LICENSE.md".to_string()));
    }

    #[test]
    fn auth_caller_always_passes_published_gate() {
        // Unpublished entry, gated universe, but caller is authenticated → pass.
        assert!(is_published_for_anon(&json!({}), false, true));
    }

    #[test]
    fn non_gated_universe_always_passes() {
        // Anonymous caller, unpublished entry, but universe is not gated → pass.
        assert!(is_published_for_anon(&json!({}), true, false));
    }

    #[test]
    fn anon_blocked_on_unpublished_gated_universe() {
        assert!(!is_published_for_anon(
            &json!({"published": false}),
            true,
            true
        ));
        assert!(!is_published_for_anon(&json!({}), true, true));
    }

    #[test]
    fn anon_allowed_on_published_gated_universe() {
        assert!(is_published_for_anon(
            &json!({"published": true}),
            true,
            true
        ));
    }

    #[test]
    fn audit_flags_only_unindexed_disk_files() {
        let indexed: std::collections::HashSet<String> = ["content/page.md", "tasks/1.md"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let on_disk = vec![
            "content/page.md".to_string(),         // indexed → safe
            "tasks/1.md".to_string(),              // indexed → safe
            "drafts/thrive-market.md".to_string(), // NOT indexed → leak surface
            "WhatsApp Image 2026.jpg".to_string(), // NOT indexed → leak surface
        ];
        let leaks = unindexed_disk_paths(&indexed, &on_disk);
        assert_eq!(
            leaks,
            vec![
                "WhatsApp Image 2026.jpg".to_string(),
                "drafts/thrive-market.md".to_string(),
            ]
        );
    }
}
