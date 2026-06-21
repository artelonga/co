//! CO-89: maintain `profile` entries from commit author identities.
//!
//! One `profile` entry per unique contributor handle. The profile carries the
//! contributor's display name, email, IANA-ish timezone (offset of their most
//! recent commit), role, and an `analytics` struct (populated by
//! [`crate::content::gitsync::analytics`]). Profiles are derived purely from the
//! parsed commit stream, so this module is unit-testable without git or a DB.

use std::collections::BTreeMap;

use serde_json::json;

use crate::content::gitsync::git_log::CommitRecord;
use crate::domain::EntryDomain;

/// Vault path for a profile entry.
pub fn profile_path(handle: &str) -> String {
    format!("profiles/{handle}.md")
}

/// Collect one [`EntryDomain`] of type `profile` per distinct author handle in
/// `records`. The `analytics` struct starts empty (`{}`) — `recompute_profile_*`
/// fills it. Display name + email + timezone come from the contributor's most
/// recent commit (records are newest-first, so the first seen wins).
pub fn build_profile_entries(universe_key: &str, records: &[CommitRecord]) -> Vec<EntryDomain> {
    // BTreeMap keeps the output deterministic (sorted by handle).
    let mut seen: BTreeMap<String, &CommitRecord> = BTreeMap::new();
    for rec in records {
        seen.entry(rec.author_handle.clone()).or_insert(rec);
    }

    seen.into_iter()
        .map(|(handle, rec)| profile_entry(universe_key, &handle, rec))
        .collect()
}

fn profile_entry(universe_key: &str, handle: &str, rec: &CommitRecord) -> EntryDomain {
    let role = role_for(handle);
    let frontmatter = json!({
        "type": "profile",
        "handle": handle,
        "title": rec.author_name,
        "display_name": rec.author_name,
        "email": rec.author_email,
        "timezone": rec.tz_offset,
        "role": role,
        "analytics": {},
    });
    let body = format!(
        "Contributor profile for **{}** (`{handle}`).",
        rec.author_name
    );
    EntryDomain {
        path: profile_path(handle),
        universe_key: universe_key.to_string(),
        entry_type: "profile".to_string(),
        title: Some(rec.author_name.clone()),
        frontmatter,
        body_hash: co::entry::Entry::hash_body(&body),
        body,
        created_at: Some(rec.committed_at.clone()),
        updated_at: Some(rec.committed_at.clone()),
    }
}

/// Default role for a handle. `yuri` is the owner; the AI co-author is an
/// `observer`; everyone else is a `contributor`. Roles are advisory metadata —
/// they don't gate access (that's the universe membership model).
fn role_for(handle: &str) -> &'static str {
    match handle {
        "yuri" => "owner",
        "claude" => "observer",
        _ => "contributor",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::gitsync::git_log::parse_git_log;
    use crate::content::gitsync::git_log::{RS, US};

    fn log() -> String {
        format!(
            "{RS}a1{US}2026-04-26T23:00:00-03:00{US}Yuri{US}yuri@artelonga.com.br\
{US}Yuri{US}yuri@artelonga.com.br{US}{US}feat: CO-1 x{US}\n 1 file changed, 1 insertion(+)\n\
{RS}a2{US}2026-04-25T10:00:00-03:00{US}Claude{US}noreply@anthropic.com\
{US}Claude{US}noreply@anthropic.com{US}{US}chore: y{US}\n 1 file changed, 1 insertion(+)\n\
{RS}a3{US}2026-04-24T10:00:00-03:00{US}Yuri{US}yuri@artelonga.com.br\
{US}Yuri{US}yuri@artelonga.com.br{US}{US}fix: z{US}\n 1 file changed, 1 insertion(+)\n"
        )
    }

    #[test]
    fn one_profile_per_handle() {
        let recs = parse_git_log(&log());
        let profiles = build_profile_entries("co-dev", &recs);
        // yuri (2 commits) collapses to one profile; claude is the other.
        assert_eq!(profiles.len(), 2);
        let handles: Vec<&str> = profiles
            .iter()
            .map(|p| p.frontmatter["handle"].as_str().unwrap())
            .collect();
        assert_eq!(handles, vec!["claude", "yuri"]); // sorted

        let yuri = profiles
            .iter()
            .find(|p| p.path == "profiles/yuri.md")
            .unwrap();
        assert_eq!(yuri.entry_type, "profile");
        assert_eq!(yuri.frontmatter["role"], "owner");
        assert_eq!(yuri.frontmatter["email"], "yuri@artelonga.com.br");

        let claude = profiles
            .iter()
            .find(|p| p.path == "profiles/claude.md")
            .unwrap();
        assert_eq!(claude.frontmatter["role"], "observer");
    }
}
