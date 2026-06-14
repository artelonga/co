//! CO-399: sala scope resolution + visibility gating.
//!
//! A "scope" is the set of universes a single sala canvas spans:
//!   · `*`        — every universe visible to the caller (the `/sala` view).
//!   · `a,b,c`    — an explicit, normalized subset (`/sala?u=a,b,c`).
//!
//! These helpers turn a scope identifier (plus the caller's identity) into the
//! concrete, *visibility-filtered* list of universes the canvas may render. They
//! are the single authority for "what is in scope" — the page route, entry
//! picker and share-token read path all funnel through them, so anonymous and
//! cross-account callers can only ever see the public slice.

use super::Storage;

/// The marker scope meaning "every universe visible to the caller".
pub const ALL_SCOPE: &str = "*";

/// Normalize a raw scope string into its canonical, deterministic form.
///
/// `*` / empty → [`ALL_SCOPE`]. Otherwise: split on commas, trim, drop empties,
/// de-duplicate and sort. The result is machine-local-state-free so the same
/// subset always hashes to the same scope key (Wave 7 cross-device sync).
pub fn normalize_scope(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == ALL_SCOPE {
        return ALL_SCOPE.to_string();
    }
    let mut keys: Vec<&str> = trimmed
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    if keys.is_empty() {
        ALL_SCOPE.to_string()
    } else {
        keys.join(",")
    }
}

/// Is this universe readable by anyone (no login, any account)? Mirrors the
/// `universe_visibility_gate` middleware's public branch.
fn is_publicly_readable(u: &crate::models::Universe) -> bool {
    u.is_public
        || u.is_template
        || u.visibility == "public-subscribable"
        || u.visibility == "public-static"
        || u.visibility == "unlisted"
}

/// A universe in scope, trimmed to the fields the canvas needs for labeling.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScopeUniverse {
    pub key: String,
    pub name: String,
    pub content_count: i64,
}

impl From<&crate::models::Universe> for ScopeUniverse {
    fn from(u: &crate::models::Universe) -> Self {
        ScopeUniverse {
            key: u.key.clone(),
            name: u.name.clone(),
            content_count: u.content_count,
        }
    }
}

impl Storage {
    /// Can `user_id` (None = anonymous) read this universe? Public/template/
    /// unlisted universes are visible to anyone; private ones require the caller
    /// to be the owner, a member, or a subscriber.
    pub fn can_view_universe(&self, key: &str, user_id: Option<&str>) -> bool {
        let Some(u) = self.get_universe(key) else {
            return false;
        };
        if is_publicly_readable(&u) {
            return true;
        }
        match user_id {
            Some(uid) => {
                u.owner_id == uid
                    || self.is_universe_member(key, uid)
                    || self.is_subscribed(uid, key)
            }
            None => false,
        }
    }

    /// Resolve a scope identifier into the caller-visible universes it covers.
    ///
    /// * [`ALL_SCOPE`] → every universe the caller can see (public set, plus the
    ///   caller's own/member/subscribed universes when logged in).
    /// * `a,b` subset → exactly those keys that exist *and* pass [`can_view_universe`].
    ///
    /// The returned list is sorted by key so the picker order is stable.
    pub fn resolve_scope_universes(
        &self,
        scope: &str,
        user_id: Option<&str>,
    ) -> Vec<ScopeUniverse> {
        let normalized = normalize_scope(scope);
        let mut universes: Vec<crate::models::Universe> = if normalized == ALL_SCOPE {
            self.list_all_visible_universes(user_id)
        } else {
            normalized
                .split(',')
                .filter(|k| self.can_view_universe(k, user_id))
                .filter_map(|k| self.get_universe(k))
                .collect()
        };
        universes.sort_by(|a, b| a.key.cmp(&b.key));
        universes.dedup_by(|a, b| a.key == b.key);
        universes.iter().map(ScopeUniverse::from).collect()
    }

    /// Every universe visible to the caller: the publicly-listed set unioned
    /// with the caller's owned/member/subscribed universes (when logged in).
    fn list_all_visible_universes(&self, user_id: Option<&str>) -> Vec<crate::models::Universe> {
        let mut out: Vec<crate::models::Universe> = self.list_public_listed_universes();
        if let Some(uid) = user_id {
            for u in self.list_universes_for_user(uid) {
                if !out.iter().any(|e| e.key == u.key) {
                    out.push(u);
                }
            }
        }
        out
    }

    /// Universes that appear in public listings (the same set the SPA hub shows
    /// anonymous visitors): public-subscribable, public-static and templates.
    /// Unlisted/private universes are intentionally excluded from the `*` view
    /// but remain reachable via an explicit `?u=` subset.
    fn list_public_listed_universes(&self) -> Vec<crate::models::Universe> {
        let mut stmt = match self.conn.prepare(
            "SELECT u.key FROM universes u \
             WHERE COALESCE(u.hidden, 0) = 0 \
               AND u.deleted_at IS NULL AND u.archived_at IS NULL \
               AND ( u.visibility = 'public-subscribable' \
                  OR u.visibility = 'public-static' \
                  OR u.is_template = 1 ) \
             ORDER BY u.key ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("list_public_listed_universes prepare: {e}");
                return Vec::new();
            }
        };
        let keys: Vec<String> = match stmt.query_map([], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(e) => {
                tracing::error!("list_public_listed_universes query: {e}");
                return Vec::new();
            }
        };
        keys.iter().filter_map(|k| self.get_universe(k)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_sorts_dedups_and_collapses_to_all() {
        assert_eq!(normalize_scope("b,a,c"), "a,b,c");
        assert_eq!(normalize_scope(" b , a , a "), "a,b");
        assert_eq!(normalize_scope("*"), "*");
        assert_eq!(normalize_scope(""), "*");
        assert_eq!(normalize_scope("  "), "*");
        assert_eq!(normalize_scope(",,"), "*");
        // Deterministic: any permutation of the same set maps to one key.
        assert_eq!(normalize_scope("x,y"), normalize_scope("y,x"));
    }
}
