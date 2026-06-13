//! Versioning — CO-native versioning: states (Phase 1), branches (Phase 2),
//! op-log time-travel/diff/branching (CO-95 phases 2-4).

pub mod branches;
pub mod op_log;
/// CO-75: replay version history to any instant + auto-changelog.
pub mod reconstruct;
pub mod states;
