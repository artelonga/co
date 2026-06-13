//! CO-54: configurable conflict-resolution strategy.
//!
//! [`detect_conflicts`](super::conflict_detector::detect_conflicts) already
//! reports a [`Conflict`] whenever local and remote diverge — for a
//! `BothModified` conflict, both sides differ from the common base (Scenario 2:
//! CLI sync vs web). This module turns a detected conflict into the concrete
//! [`Action`] dictated by a configured strategy, so a non-interactive client
//! (`co sync push`, the Obsidian plugin) can resolve automatically instead of
//! parking every conflict for a human.
//!
//! The default — and the one named in the spec — is **last-write-wins by
//! timestamp**: the revision with the newer `updated_at` is kept.

use super::conflict_detector::{Conflict, ConflictKind, EntryRevision};
use super::conflict_resolver::Action;

/// How to auto-resolve a detected conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ConflictStrategy {
    /// Keep whichever side has the newer `updated_at` timestamp (spec default).
    #[default]
    LastWriteWins,
    /// Always keep the local revision.
    PreferLocal,
    /// Always take the remote revision.
    PreferRemote,
    /// Never overwrite — keep both (local renamed) so a human can reconcile.
    KeepBoth,
}

impl std::str::FromStr for ConflictStrategy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().replace('-', "_").as_str() {
            "last_write_wins" | "lww" | "" => Ok(ConflictStrategy::LastWriteWins),
            "prefer_local" | "local" => Ok(ConflictStrategy::PreferLocal),
            "prefer_remote" | "remote" => Ok(ConflictStrategy::PreferRemote),
            "keep_both" | "both" => Ok(ConflictStrategy::KeepBoth),
            other => Err(format!("unknown ConflictStrategy: {other}")),
        }
    }
}

/// Decide the resolution [`Action`] for `conflict` under `strategy`.
///
/// `Replace` means *remote wins* (overwrite local); `KeepLocal` means *local
/// wins*. For the delete-vs-modify kinds the default leans toward **no data
/// loss** (the modification beats the deletion) regardless of strategy, since
/// resurrecting content is recoverable while a wrong delete is not.
pub fn resolve(conflict: &Conflict, strategy: ConflictStrategy) -> Action {
    match conflict.kind {
        // A one-sided new entry isn't a real divergence — keep whichever side
        // has it. (`detect_conflicts` still surfaces these so callers can log.)
        ConflictKind::LocalOnlyNew => Action::KeepLocal,
        ConflictKind::RemoteOnlyNew => Action::Replace,
        // Modification beats deletion — never silently drop edited content.
        ConflictKind::LocalDeletedRemoteModified => Action::Replace,
        ConflictKind::LocalModifiedRemoteDeleted => Action::KeepLocal,
        // The real contended case: both sides edited the same path.
        ConflictKind::BothModified => match strategy {
            ConflictStrategy::PreferLocal => Action::KeepLocal,
            ConflictStrategy::PreferRemote => Action::Replace,
            ConflictStrategy::KeepBoth => Action::KeepBoth,
            ConflictStrategy::LastWriteWins => last_write_wins(&conflict.local, &conflict.remote),
        },
    }
}

/// Compare `updated_at` timestamps and keep the newer side. Remote strictly
/// newer ⇒ `Replace`; otherwise (local newer, equal, or unparseable) keep local
/// — the conservative tiebreak that never overwrites on ambiguity.
fn last_write_wins(local: &EntryRevision, remote: &EntryRevision) -> Action {
    match (parse_ts(&local.updated_at), parse_ts(&remote.updated_at)) {
        (Some(l), Some(r)) if r > l => Action::Replace,
        // Remote has a timestamp but local doesn't → treat remote as newer.
        (None, Some(_)) => Action::Replace,
        _ => Action::KeepLocal,
    }
}

/// Parse an optional RFC-3339 timestamp into a comparable epoch (nanoseconds).
fn parse_ts(ts: &Option<String>) -> Option<i64> {
    let s = ts.as_deref()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_nanos_opt().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(source: &str, updated_at: Option<&str>) -> EntryRevision {
        EntryRevision {
            path: "content/sobre.md".into(),
            body_hash: format!("hash-{source}"),
            body: Some(format!("body from {source}")),
            updated_at: updated_at.map(String::from),
            source: source.into(),
        }
    }

    fn both_modified(local: EntryRevision, remote: EntryRevision) -> Conflict {
        Conflict::new(
            "u1",
            "content/sobre.md",
            local,
            remote,
            None,
            ConflictKind::BothModified,
        )
    }

    #[test]
    fn strategy_parses_aliases() {
        assert_eq!(
            "last-write-wins".parse::<ConflictStrategy>().unwrap(),
            ConflictStrategy::LastWriteWins
        );
        assert_eq!(
            "".parse::<ConflictStrategy>().unwrap(),
            ConflictStrategy::LastWriteWins
        );
        assert_eq!(
            "remote".parse::<ConflictStrategy>().unwrap(),
            ConflictStrategy::PreferRemote
        );
        assert!("nonsense".parse::<ConflictStrategy>().is_err());
    }

    #[test]
    fn lww_remote_newer_replaces_local() {
        let c = both_modified(
            rev("local", Some("2026-06-13T10:00:00Z")),
            rev("remote", Some("2026-06-13T11:00:00Z")),
        );
        assert_eq!(
            resolve(&c, ConflictStrategy::LastWriteWins),
            Action::Replace
        );
    }

    #[test]
    fn lww_local_newer_keeps_local() {
        let c = both_modified(
            rev("local", Some("2026-06-13T12:00:00Z")),
            rev("remote", Some("2026-06-13T11:00:00Z")),
        );
        assert_eq!(
            resolve(&c, ConflictStrategy::LastWriteWins),
            Action::KeepLocal
        );
    }

    #[test]
    fn lww_equal_timestamps_keep_local_no_overwrite() {
        let ts = Some("2026-06-13T12:00:00Z");
        let c = both_modified(rev("local", ts), rev("remote", ts));
        assert_eq!(
            resolve(&c, ConflictStrategy::LastWriteWins),
            Action::KeepLocal
        );
    }

    #[test]
    fn explicit_prefer_strategies_override_timestamps() {
        // Remote is newer, but PreferLocal must still keep local.
        let c = both_modified(
            rev("local", Some("2026-06-13T10:00:00Z")),
            rev("remote", Some("2026-06-13T11:00:00Z")),
        );
        assert_eq!(
            resolve(&c, ConflictStrategy::PreferLocal),
            Action::KeepLocal
        );
        assert_eq!(resolve(&c, ConflictStrategy::PreferRemote), Action::Replace);
        assert_eq!(resolve(&c, ConflictStrategy::KeepBoth), Action::KeepBoth);
    }

    #[test]
    fn modification_beats_deletion_both_directions() {
        let modremote = Conflict::new(
            "u1",
            "p.md",
            rev("local", None),
            rev("remote", Some("2026-06-13T11:00:00Z")),
            None,
            ConflictKind::LocalDeletedRemoteModified,
        );
        assert_eq!(
            resolve(&modremote, ConflictStrategy::LastWriteWins),
            Action::Replace
        );

        let modlocal = Conflict::new(
            "u1",
            "p.md",
            rev("local", Some("2026-06-13T11:00:00Z")),
            rev("remote", None),
            None,
            ConflictKind::LocalModifiedRemoteDeleted,
        );
        assert_eq!(
            resolve(&modlocal, ConflictStrategy::LastWriteWins),
            Action::KeepLocal
        );
    }
}
