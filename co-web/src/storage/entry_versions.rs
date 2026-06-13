//! CO-54: `entry_versions` audit trail — pre-overwrite snapshots + history.
//!
//! Every `PUT` on an entry snapshots the *previous* content here before the new
//! content is written to disk and re-indexed. This gives us three guarantees
//! from the spec:
//!
//! * **Data-loss prevention** — the prior version is durably stored before any
//!   destructive overwrite, so a crash mid-write never loses committed data.
//! * **Audit trail** — `actor` + `timestamp` + `hash` per version.
//! * **Manual recovery** — `GET …/entries/versions` reads [`list_entry_versions`].
//!
//! Idempotency: [`save_entry_version`] is a no-op when the latest stored version
//! already has the same `hash` (re-applying the identical write does not grow
//! the history). Retention: each entry keeps the **50 newest versions or 90
//! days**, whichever is more generous.

use chrono::Utc;
use rusqlite::{Result as SqlResult, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::Storage;

/// Keep at least this many recent versions per entry, regardless of age.
const RETENTION_KEEP_VERSIONS: i64 = 50;
/// Keep versions newer than this, regardless of count.
const RETENTION_KEEP_DAYS: i64 = 90;

/// SHA-256 over the *full* version content (frontmatter + body).
///
/// Unlike `entry.body_hash` (which hashes the body only), this covers
/// frontmatter too, so a frontmatter-only edit produces a distinct hash. The
/// idempotency guard in [`Storage::save_entry_version`] relies on that.
pub fn version_content_hash(frontmatter_json: &str, body: &str) -> String {
    let mut h = Sha256::new();
    h.update(frontmatter_json.as_bytes());
    h.update(b"\n");
    h.update(body.as_bytes());
    format!("{:x}", h.finalize())
}

/// One row from the `entry_versions` table (newest-first in API responses).
#[derive(Debug, Clone, Serialize)]
pub struct EntryVersionRow {
    pub id: i64,
    pub universe_key: String,
    pub path: String,
    pub version: i64,
    pub body: String,
    /// Serialized frontmatter JSON (string, so the API can round-trip it).
    pub frontmatter_json: String,
    pub hash: String,
    pub actor: Option<String>,
    pub timestamp: String,
}

impl Storage {
    /// Snapshot a version of an entry into the audit trail.
    ///
    /// Returns `Ok(Some(version))` for a freshly inserted version, or `Ok(None)`
    /// when the write is idempotent (the latest stored version already has the
    /// same content `hash`). Pruning to the retention window runs after a real
    /// insert. The `hash` is computed internally over frontmatter + body.
    pub fn save_entry_version(
        &self,
        universe_key: &str,
        path: &str,
        body: &str,
        frontmatter_json: &str,
        actor: Option<&str>,
    ) -> SqlResult<Option<i64>> {
        let hash = version_content_hash(frontmatter_json, body);
        // Latest (version, hash) for this entry, if any.
        let latest: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT version, hash FROM entry_versions \
                 WHERE universe_key = ?1 AND path = ?2 \
                 ORDER BY version DESC LIMIT 1",
                params![universe_key, path],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .ok();

        // Idempotent: identical content already at the head of the history.
        if let Some((_, ref latest_hash)) = latest
            && *latest_hash == hash
        {
            return Ok(None);
        }

        let next_version = latest.map(|(v, _)| v + 1).unwrap_or(1);
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            "INSERT INTO entry_versions \
             (universe_key, path, version, body, frontmatter_json, hash, actor, timestamp) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                universe_key,
                path,
                next_version,
                body,
                frontmatter_json,
                hash,
                actor,
                now
            ],
        )?;

        self.prune_entry_versions(universe_key, path, next_version)?;
        Ok(Some(next_version))
    }

    /// Drop versions that are **both** outside the newest-N window **and** older
    /// than the day cutoff. A version is retained if it satisfies *either* rule.
    fn prune_entry_versions(
        &self,
        universe_key: &str,
        path: &str,
        latest_version: i64,
    ) -> SqlResult<()> {
        let version_floor = latest_version - RETENTION_KEEP_VERSIONS;
        if version_floor <= 0 {
            // Fewer than the keep-count exist; nothing can be old enough to drop.
            return Ok(());
        }
        let date_cutoff = (Utc::now() - chrono::Duration::days(RETENTION_KEEP_DAYS)).to_rfc3339();
        self.conn.execute(
            "DELETE FROM entry_versions \
             WHERE universe_key = ?1 AND path = ?2 \
               AND version <= ?3 AND timestamp < ?4",
            params![universe_key, path, version_floor, date_cutoff],
        )?;
        Ok(())
    }

    /// Version history for an entry, newest first, capped at `limit`.
    pub fn list_entry_versions(
        &self,
        universe_key: &str,
        path: &str,
        limit: usize,
    ) -> SqlResult<Vec<EntryVersionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, universe_key, path, version, body, frontmatter_json, hash, actor, timestamp \
             FROM entry_versions \
             WHERE universe_key = ?1 AND path = ?2 \
             ORDER BY version DESC LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![universe_key, path, limit as i64], |row| {
                Ok(EntryVersionRow {
                    id: row.get(0)?,
                    universe_key: row.get(1)?,
                    path: row.get(2)?,
                    version: row.get(3)?,
                    body: row.get(4)?,
                    frontmatter_json: row.get(5)?,
                    hash: row.get(6)?,
                    actor: row.get(7)?,
                    timestamp: row.get(8)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> Storage {
        let dir = tempfile::tempdir().unwrap();
        Storage::new(dir.path())
    }

    #[test]
    fn first_save_is_version_1() {
        let s = storage();
        let v = s
            .save_entry_version("u", "content/a.md", "body1", "{}", Some("alice"))
            .unwrap();
        assert_eq!(v, Some(1));
    }

    #[test]
    fn distinct_content_bumps_version() {
        let s = storage();
        s.save_entry_version("u", "p.md", "b1", "{}", None).unwrap();
        let v2 = s.save_entry_version("u", "p.md", "b2", "{}", None).unwrap();
        assert_eq!(v2, Some(2));
        let history = s.list_entry_versions("u", "p.md", 10).unwrap();
        assert_eq!(history.len(), 2);
        // newest first
        assert_eq!(history[0].version, 2);
        assert_eq!(history[0].hash, version_content_hash("{}", "b2"));
        assert_eq!(history[1].version, 1);
    }

    #[test]
    fn frontmatter_only_change_is_a_distinct_version() {
        let s = storage();
        // Same body, different frontmatter — the content hash must differ so the
        // version still bumps (web field edits change frontmatter, not body).
        s.save_entry_version("u", "p.md", "body", "{\"a\":1}", None)
            .unwrap();
        let v2 = s
            .save_entry_version("u", "p.md", "body", "{\"a\":2}", None)
            .unwrap();
        assert_eq!(v2, Some(2));
    }

    #[test]
    fn identical_content_is_idempotent_noop() {
        let s = storage();
        s.save_entry_version("u", "p.md", "b1", "{}", None).unwrap();
        // Same body + frontmatter at the head → no new version.
        let again = s.save_entry_version("u", "p.md", "b1", "{}", None).unwrap();
        assert_eq!(again, None);
        assert_eq!(s.list_entry_versions("u", "p.md", 10).unwrap().len(), 1);
    }

    #[test]
    fn versions_are_scoped_per_entry_and_universe() {
        let s = storage();
        s.save_entry_version("u1", "a.md", "x", "{}", None).unwrap();
        s.save_entry_version("u2", "a.md", "x", "{}", None).unwrap();
        s.save_entry_version("u1", "b.md", "x", "{}", None).unwrap();
        assert_eq!(s.list_entry_versions("u1", "a.md", 10).unwrap().len(), 1);
        assert_eq!(s.list_entry_versions("u2", "a.md", 10).unwrap().len(), 1);
        // Each (universe, path) numbers from 1 independently.
        assert_eq!(
            s.list_entry_versions("u2", "a.md", 10).unwrap()[0].version,
            1
        );
    }

    #[test]
    fn recent_versions_retained_even_past_keep_count() {
        let s = storage();
        // 60 distinct writes; all are recent (well within 90 days), so the
        // "or 90 days" rule keeps every one of them despite exceeding 50.
        for i in 0..60 {
            s.save_entry_version("u", "p.md", &format!("b{i}"), "{}", None)
                .unwrap();
        }
        let history = s.list_entry_versions("u", "p.md", 1000).unwrap();
        assert_eq!(history.len(), 60);
        assert_eq!(history[0].version, 60);
    }
}
