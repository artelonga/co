//! CO-448 — token capability scopes (hybrid: capabilities + named bundles).
//!
//! An API token can carry a least-privilege set of **capabilities** — strings
//! of the form `recurso:ação` (e.g. `entries:read`, `chat:write`). To keep
//! issuance ergonomic the model is **hybrid**: an issuer may pass either raw
//! capabilities *or* a named **bundle** (`read`/`write`/`admin`/`agent`) that
//! expands to a set of capabilities. Expansion happens once, at issuance, and
//! the expanded set is what gets persisted in `api_tokens.scopes` — so a token
//! is **auditable** (you can read exactly what it can do) and the check path
//! never has to re-resolve bundles.
//!
//! `NULL` scopes (every token issued before CO-448, plus the legacy
//! `create_api_token` path) means **no declared scope → inherit the owner's
//! tier** — the all-or-nothing behavior from before. Only tokens *with* a
//! non-NULL `scopes` value are restricted, so nothing already issued breaks.

use std::collections::BTreeSet;

/// Initial capability vocabulary (extensible — new surfaces add capabilities
/// additively as the roadmap grows, e.g. CO-288 cost panel → `deployments:read`).
///
/// Used to validate issuance input: an unknown capability string is rejected so
/// a typo can't silently mint a token that grants nothing (or, worse, that a
/// future capability rename would suddenly widen).
pub const KNOWN_CAPABILITIES: &[&str] = &[
    // Reads
    "entries:read",
    "universes:read",
    "telemetry:read",
    "gestao:read",
    "funnel:read",
    "chat:read",
    "deployments:read",
    // Writes
    "entries:write",
    "universes:write",
    "chat:write",
    // Agent
    "agent:dispatch",
];

/// Named bundles (presets) → their capability sets. Higher bundles build on
/// lower ones (`write ⊇ read`, `admin ⊇ write`, `agent ⊇ write`).
const BUNDLE_READ: &[&str] = &["entries:read", "universes:read", "telemetry:read"];
const BUNDLE_WRITE_EXTRA: &[&str] = &["entries:write", "universes:write"];
const BUNDLE_ADMIN_EXTRA: &[&str] = &[
    "gestao:read",
    "funnel:read",
    "chat:read",
    "chat:write",
    "deployments:read",
];
const BUNDLE_AGENT_EXTRA: &[&str] = &["agent:dispatch"];

/// Expand a single bundle name to its capability set, or `None` if the name is
/// not a known bundle.
fn bundle_capabilities(name: &str) -> Option<BTreeSet<String>> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    match name {
        "read" => {
            set.extend(BUNDLE_READ.iter().map(|s| s.to_string()));
        }
        "write" => {
            set.extend(BUNDLE_READ.iter().map(|s| s.to_string()));
            set.extend(BUNDLE_WRITE_EXTRA.iter().map(|s| s.to_string()));
        }
        "admin" => {
            set.extend(BUNDLE_READ.iter().map(|s| s.to_string()));
            set.extend(BUNDLE_WRITE_EXTRA.iter().map(|s| s.to_string()));
            set.extend(BUNDLE_ADMIN_EXTRA.iter().map(|s| s.to_string()));
        }
        "agent" => {
            set.extend(BUNDLE_READ.iter().map(|s| s.to_string()));
            set.extend(BUNDLE_WRITE_EXTRA.iter().map(|s| s.to_string()));
            set.extend(BUNDLE_AGENT_EXTRA.iter().map(|s| s.to_string()));
        }
        _ => return None,
    }
    Some(set)
}

/// Resolve an issuance request (a mix of bundle names and raw `recurso:ação`
/// capabilities) into the canonical, deduplicated, sorted capability set that
/// gets persisted.
///
/// Returns `Err(invalid)` listing every entry that is neither a known bundle
/// nor a known capability, so a typo fails loudly at issuance rather than
/// minting a misleading token.
pub fn resolve_scopes(requested: &[String]) -> Result<BTreeSet<String>, Vec<String>> {
    let mut resolved: BTreeSet<String> = BTreeSet::new();
    let mut invalid: Vec<String> = Vec::new();

    for raw in requested {
        let item = raw.trim();
        if item.is_empty() {
            continue;
        }
        if let Some(bundle) = bundle_capabilities(item) {
            resolved.extend(bundle);
        } else if KNOWN_CAPABILITIES.contains(&item) {
            resolved.insert(item.to_string());
        } else {
            invalid.push(item.to_string());
        }
    }

    if invalid.is_empty() {
        Ok(resolved)
    } else {
        Err(invalid)
    }
}

/// Does a resolved capability set grant the `required` capability?
///
/// Plain set membership — bundles are already expanded at issuance, so the
/// check is a simple, fast containment test with no escalation path: a token
/// never obtains a capability outside its persisted set.
pub fn grants(resolved: &BTreeSet<String>, required: &str) -> bool {
    resolved.contains(required)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Acceptance (d): bundles expand to the documented capability sets, and the
    /// hierarchy holds (`write ⊇ read`, `admin ⊇ write`, `agent ⊇ write`).
    #[test]
    fn bundles_expand_to_documented_sets() {
        let read = resolve_scopes(&["read".to_string()]).unwrap();
        assert_eq!(
            read,
            set(&["entries:read", "universes:read", "telemetry:read"])
        );

        let write = resolve_scopes(&["write".to_string()]).unwrap();
        assert!(read.is_subset(&write), "write must contain read");
        assert!(grants(&write, "entries:write"));
        assert!(grants(&write, "universes:write"));

        let admin = resolve_scopes(&["admin".to_string()]).unwrap();
        assert!(write.is_subset(&admin), "admin must contain write");
        for cap in [
            "gestao:read",
            "funnel:read",
            "chat:read",
            "chat:write",
            "deployments:read",
        ] {
            assert!(grants(&admin, cap), "admin must grant {cap}");
        }

        let agent = resolve_scopes(&["agent".to_string()]).unwrap();
        assert!(write.is_subset(&agent), "agent must contain write");
        assert!(grants(&agent, "agent:dispatch"));
        // agent does NOT imply admin surfaces.
        assert!(!grants(&agent, "gestao:read"));
    }

    /// Hybrid: a request can mix a bundle and raw capabilities; the union is
    /// resolved and deduplicated.
    #[test]
    fn mixes_bundle_and_raw_capabilities() {
        let resolved = resolve_scopes(&[
            "read".to_string(),
            "chat:read".to_string(),
            "entries:read".to_string(),
        ])
        .unwrap();
        // read bundle + chat:read, entries:read already in read → deduped.
        let mut expected = set(&["entries:read", "universes:read", "telemetry:read"]);
        expected.insert("chat:read".to_string());
        assert_eq!(resolved, expected);
    }

    /// An unknown capability / bundle name fails loudly at issuance.
    #[test]
    fn rejects_unknown_capability() {
        let err =
            resolve_scopes(&["entries:read".to_string(), "bogus:thing".to_string()]).unwrap_err();
        assert_eq!(err, vec!["bogus:thing".to_string()]);
    }

    /// Blank / whitespace-only entries are skipped, not treated as invalid.
    #[test]
    fn skips_blank_entries() {
        let resolved =
            resolve_scopes(&["".to_string(), "  ".to_string(), "chat:read".to_string()]).unwrap();
        assert_eq!(resolved, set(&["chat:read"]));
    }

    /// `grants` is plain membership — no escalation from a narrower set.
    #[test]
    fn grants_is_membership_no_escalation() {
        let read = resolve_scopes(&["read".to_string()]).unwrap();
        assert!(grants(&read, "entries:read"));
        assert!(
            !grants(&read, "entries:write"),
            "read must not escalate to write"
        );
        assert!(!grants(&read, "chat:read"));
    }
}
