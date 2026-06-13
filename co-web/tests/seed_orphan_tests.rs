//! CO-438 (Bug 1): the admin-content seed used to INSERT a `universes` row for
//! the importable *private* universe `claude-code` owned by the sentinel
//! 'system'. `check_universe_access` admits a caller only when it is the
//! `owner_id` (or a member), and 'system' is no real user — so the row was an
//! orphan: `POST /universes` → 409 "key taken", yet `GET /universes/{key}` →
//! 404. The CO-429 reconcile that flipped claude-code to `private` is what
//! tipped a (GET-able) public-subscribable row into the orphan state.
//!
//! Fix: don't seed importable private universes — let `co source add` create
//! them as the importing user (fully provisioned) — and never privatize a
//! system-owned row. After a seed, claude-code is either GET-able or absent,
//! never "taken but 404".

use tempfile::tempdir;

use co_web::models::UniverseAccess;
use co_web::storage::Storage;

/// Fresh seed: claude-code is NOT created (absent), so a later `co source add`
/// can create it owned by a real user. Never the system-owned orphan.
#[test]
fn seed_does_not_create_orphan_claude_code() {
    let data_dir = tempdir().unwrap();
    let mut storage = Storage::new(data_dir.path());
    storage.seed_admin_content_universes();

    assert!(
        storage.get_universe("claude-code").is_none(),
        "importable private universe must not be seeded (would orphan)"
    );
}

/// Re-running the seed stays a no-op: still absent.
#[test]
fn seed_is_idempotent_on_redeploy() {
    let data_dir = tempdir().unwrap();
    let mut storage = Storage::new(data_dir.path());
    storage.seed_admin_content_universes();
    storage.seed_admin_content_universes();

    assert!(storage.get_universe("claude-code").is_none());
}

/// A legacy system-owned, public-subscribable claude-code row (from the CO-364
/// seed) must NOT be flipped to private — that flip is what orphaned it. It
/// stays public-subscribable, i.e. still GET-able (MetadataOnly for anon).
#[test]
fn seed_does_not_privatize_system_owned_row() {
    let data_dir = tempdir().unwrap();
    let mut storage = Storage::new(data_dir.path());
    storage.seed_admin_content_universes();

    // Simulate the legacy CO-364 row: system-owned, public-subscribable.
    storage
        .conn()
        .execute(
            "INSERT INTO universes (key, name, description, owner_id, created_at, \
             is_template, is_public, visibility) \
             VALUES ('claude-code', 'Claude Code', '', 'system', '2026-01-01T00:00:00Z', \
             0, 1, 'public-subscribable')",
            [],
        )
        .unwrap();

    storage.seed_admin_content_universes();

    let uni = storage.get_universe("claude-code").unwrap();
    assert_eq!(
        uni.visibility, "public-subscribable",
        "a system-owned row must stay public-subscribable, not be orphaned to private"
    );
    // Public-subscribable ⇒ reachable (not the "taken but 404" denial).
    assert_ne!(
        storage.check_universe_access(None, "claude-code"),
        UniverseAccess::Denied
    );
}

/// A real-user-owned private claude-code (what `co source add` produces) stays
/// fully accessible to its owner after the seed runs — never orphaned.
#[test]
fn seed_leaves_real_owner_private_import_accessible() {
    let data_dir = tempdir().unwrap();
    let mut storage = Storage::new(data_dir.path());

    storage
        .create_universe(
            co_web::models::CreateUniverse {
                key: "claude-code".into(),
                name: "Claude Code".into(),
                description: String::new(),
            },
            "usr_yuri",
        )
        .unwrap();
    assert_eq!(
        storage.get_universe("claude-code").unwrap().visibility,
        "private"
    );

    storage.seed_admin_content_universes();

    assert_eq!(
        storage.check_universe_access(Some("usr_yuri"), "claude-code"),
        UniverseAccess::ReadWrite,
        "owner must keep access to their private import across re-deploys"
    );
}
