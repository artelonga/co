//! CO-385: Pure conflict detection for cross-device sync.
//!
//! `detect_conflicts` compares local vs remote entry sets and classifies
//! divergences into five `ConflictKind` variants. Same-hash pairs are skipped
//! (the hash-skip optimization from CO-385).

use std::collections::HashMap;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::eda::new_ulid;

/// A snapshot of an entry revision (local or remote).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntryRevision {
    pub path: String,
    pub body_hash: String,
    /// Optional body content — used by the `Update` (3-way merge) action.
    pub body: Option<String>,
    pub updated_at: Option<String>,
    /// "local" or a remote deployment identifier.
    pub source: String,
}

/// Five-way classification of how local and remote diverged.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    BothModified,
    LocalOnlyNew,
    RemoteOnlyNew,
    LocalDeletedRemoteModified,
    LocalModifiedRemoteDeleted,
}

impl std::fmt::Display for ConflictKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ConflictKind::BothModified => "both_modified",
            ConflictKind::LocalOnlyNew => "local_only_new",
            ConflictKind::RemoteOnlyNew => "remote_only_new",
            ConflictKind::LocalDeletedRemoteModified => "local_deleted_remote_modified",
            ConflictKind::LocalModifiedRemoteDeleted => "local_modified_remote_deleted",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for ConflictKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "both_modified" => Ok(ConflictKind::BothModified),
            "local_only_new" => Ok(ConflictKind::LocalOnlyNew),
            "remote_only_new" => Ok(ConflictKind::RemoteOnlyNew),
            "local_deleted_remote_modified" => Ok(ConflictKind::LocalDeletedRemoteModified),
            "local_modified_remote_deleted" => Ok(ConflictKind::LocalModifiedRemoteDeleted),
            other => Err(format!("unknown ConflictKind: {other}")),
        }
    }
}

/// A detected conflict between a local and remote entry revision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conflict {
    /// ULID — primary key for the `sync_conflicts` row.
    pub id: String,
    pub universe_key: String,
    pub path: String,
    pub local: EntryRevision,
    pub remote: EntryRevision,
    /// Common ancestor for 3-way merge (`Update` action).
    pub common_ancestor: Option<EntryRevision>,
    pub kind: ConflictKind,
    pub detected_at: String,
}

impl Conflict {
    pub fn new(
        universe_key: impl Into<String>,
        path: impl Into<String>,
        local: EntryRevision,
        remote: EntryRevision,
        common_ancestor: Option<EntryRevision>,
        kind: ConflictKind,
    ) -> Self {
        Self {
            id: new_ulid(),
            universe_key: universe_key.into(),
            path: path.into(),
            local,
            remote,
            common_ancestor,
            kind,
            detected_at: Utc::now().to_rfc3339(),
        }
    }
}

/// Classify local vs remote entry sets into conflicts.
///
/// Only entries where `body_hash` differs are returned — same-hash pairs are
/// silently skipped (hash-skip optimization from CO-385).
///
/// `local_entries` and `remote_entries` share the same `path` key space.
/// An empty `body_hash` means the entry was deleted on that side.
pub fn detect_conflicts(
    universe_key: &str,
    local_entries: &[EntryRevision],
    remote_entries: &[EntryRevision],
) -> Vec<Conflict> {
    let local_map: HashMap<&str, &EntryRevision> =
        local_entries.iter().map(|e| (e.path.as_str(), e)).collect();
    let remote_map: HashMap<&str, &EntryRevision> = remote_entries
        .iter()
        .map(|e| (e.path.as_str(), e))
        .collect();

    let mut conflicts = Vec::new();

    // Paths present in local — compare against remote.
    for (path, local) in &local_map {
        if let Some(remote) = remote_map.get(path) {
            // Hash-skip: same hash → no conflict.
            if local.body_hash == remote.body_hash {
                continue;
            }
            let kind = if local.body_hash.is_empty() {
                ConflictKind::LocalDeletedRemoteModified
            } else if remote.body_hash.is_empty() {
                ConflictKind::LocalModifiedRemoteDeleted
            } else {
                ConflictKind::BothModified
            };
            conflicts.push(Conflict::new(
                universe_key,
                *path,
                (*local).clone(),
                (*remote).clone(),
                None,
                kind,
            ));
        } else {
            // Local has it, remote doesn't.
            conflicts.push(Conflict::new(
                universe_key,
                *path,
                (*local).clone(),
                EntryRevision {
                    path: path.to_string(),
                    body_hash: String::new(),
                    body: None,
                    updated_at: None,
                    source: "remote".into(),
                },
                None,
                ConflictKind::LocalOnlyNew,
            ));
        }
    }

    // Paths only in remote.
    for (path, remote) in &remote_map {
        if !local_map.contains_key(path) {
            conflicts.push(Conflict::new(
                universe_key,
                *path,
                EntryRevision {
                    path: path.to_string(),
                    body_hash: String::new(),
                    body: None,
                    updated_at: None,
                    source: "local".into(),
                },
                (*remote).clone(),
                None,
                ConflictKind::RemoteOnlyNew,
            ));
        }
    }

    conflicts
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(path: &str, hash: &str, source: &str) -> EntryRevision {
        EntryRevision {
            path: path.into(),
            body_hash: hash.into(),
            body: None,
            updated_at: None,
            source: source.into(),
        }
    }

    #[test]
    fn same_hash_is_skipped() {
        let local = vec![rev("notes/foo.md", "abc", "local")];
        let remote = vec![rev("notes/foo.md", "abc", "remote")];
        let conflicts = detect_conflicts("u1", &local, &remote);
        assert!(
            conflicts.is_empty(),
            "same hash must not produce a conflict"
        );
    }

    #[test]
    fn both_modified_detected() {
        let local = vec![rev("notes/foo.md", "aaa", "local")];
        let remote = vec![rev("notes/foo.md", "bbb", "remote")];
        let conflicts = detect_conflicts("u1", &local, &remote);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::BothModified);
    }

    #[test]
    fn local_only_new_detected() {
        let local = vec![rev("notes/foo.md", "aaa", "local")];
        let remote: Vec<EntryRevision> = vec![];
        let conflicts = detect_conflicts("u1", &local, &remote);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::LocalOnlyNew);
    }

    #[test]
    fn remote_only_new_detected() {
        let local: Vec<EntryRevision> = vec![];
        let remote = vec![rev("notes/bar.md", "bbb", "remote")];
        let conflicts = detect_conflicts("u1", &local, &remote);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::RemoteOnlyNew);
    }

    #[test]
    fn local_deleted_remote_modified() {
        let local = vec![rev("notes/foo.md", "", "local")];
        let remote = vec![rev("notes/foo.md", "bbb", "remote")];
        let conflicts = detect_conflicts("u1", &local, &remote);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::LocalDeletedRemoteModified);
    }

    #[test]
    fn local_modified_remote_deleted() {
        let local = vec![rev("notes/foo.md", "aaa", "local")];
        let remote = vec![rev("notes/foo.md", "", "remote")];
        let conflicts = detect_conflicts("u1", &local, &remote);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].kind, ConflictKind::LocalModifiedRemoteDeleted);
    }

    #[test]
    fn conflict_has_ulid_id() {
        let local = vec![rev("notes/foo.md", "aaa", "local")];
        let remote = vec![rev("notes/foo.md", "bbb", "remote")];
        let conflicts = detect_conflicts("u1", &local, &remote);
        assert_eq!(conflicts[0].id.len(), 26);
    }

    #[test]
    fn conflict_kind_roundtrips_display() {
        let kinds = [
            ConflictKind::BothModified,
            ConflictKind::LocalOnlyNew,
            ConflictKind::RemoteOnlyNew,
            ConflictKind::LocalDeletedRemoteModified,
            ConflictKind::LocalModifiedRemoteDeleted,
        ];
        for kind in &kinds {
            let s = kind.to_string();
            let back: ConflictKind = s.parse().expect("roundtrip");
            assert_eq!(&back, kind);
        }
    }
}
