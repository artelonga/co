use super::support::*;

#[test]
fn test_access_template_anonymous() {
    let (storage, _dir) = make_storage();
    set_visibility(&storage, "default", "template");
    let access = storage.check_universe_access(None, "default");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
}

/// 1. Template universe → READ for logged-in user too.
#[test]
fn test_access_template_logged_in() {
    let (storage, _dir) = make_storage();
    set_visibility(&storage, "default", "template");
    let access = storage.check_universe_access(Some("some-user"), "default");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
}

/// 2. Owner → READ+WRITE regardless of visibility.
#[test]
fn test_access_owner_readwrite() {
    let (mut storage, _dir) = make_storage();
    // "default" universe is owned by "system"; create one owned by test-owner.
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "my-uni".into(),
                name: "My Universe".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    let access = storage.check_universe_access(Some("owner-1"), "my-uni");
    assert_eq!(access, crate::models::UniverseAccess::ReadWrite);
}

/// 3. Member with editor role → READ+WRITE.
#[test]
fn test_access_editor_member_readwrite() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "collab".into(),
                name: "Collab".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    storage
        .add_universe_member("collab", "editor-1", "editor")
        .unwrap();
    let access = storage.check_universe_access(Some("editor-1"), "collab");
    assert_eq!(access, crate::models::UniverseAccess::ReadWrite);
}

/// 4. Member with viewer role → READ only.
#[test]
fn test_access_viewer_member_readonly() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "readonly-uni".into(),
                name: "Read Only".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    storage
        .add_universe_member("readonly-uni", "viewer-1", "viewer")
        .unwrap();
    let access = storage.check_universe_access(Some("viewer-1"), "readonly-uni");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
}

/// 5. Subscribed user → READ only.
#[test]
fn test_access_subscribed_readonly() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "pub-uni".into(),
                name: "Public".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "pub-uni", "public-subscribable");
    storage.subscribe_universe("sub-user", "pub-uni").unwrap();
    let access = storage.check_universe_access(Some("sub-user"), "pub-uni");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
}

/// 6. Public-subscribable universe → MetadataOnly for non-subscribed anonymous.
#[test]
fn test_access_public_subscribable_anonymous_metadata_only() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "disco".into(),
                name: "Discoverable".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "disco", "public-subscribable");
    // Anonymous (no user_id)
    let access = storage.check_universe_access(None, "disco");
    assert_eq!(access, crate::models::UniverseAccess::MetadataOnly);
}

/// 1.46.0: public-subscribable → ReadOnly for any logged-in user
/// (including non-subscribers). Anonymous still gets MetadataOnly. The
/// pre-collapse behavior gated content behind subscription; the new
/// model treats any authed user as eligible to read.
#[test]
fn test_access_public_subscribable_logged_in_not_subscribed() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "disco2".into(),
                name: "Discoverable2".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "disco2", "public-subscribable");
    let access = storage.check_universe_access(Some("other-user"), "disco2");
    assert_eq!(access, crate::models::UniverseAccess::ReadOnly);
    // Anonymous still sees only metadata.
    let anon = storage.check_universe_access(None, "disco2");
    assert_eq!(anon, crate::models::UniverseAccess::MetadataOnly);
}

/// 7. Private universe → Denied for non-owner.
#[test]
fn test_access_private_denied_to_non_owner() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "secret".into(),
                name: "Secret".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    let access = storage.check_universe_access(Some("attacker"), "secret");
    assert_eq!(access, crate::models::UniverseAccess::Denied);
}

/// 7. Private universe → Denied for anonymous user.
#[test]
fn test_access_private_denied_anonymous() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "secret2".into(),
                name: "Secret2".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    let access = storage.check_universe_access(None, "secret2");
    assert_eq!(access, crate::models::UniverseAccess::Denied);
}

/// Non-existent universe → Denied.
#[test]
fn test_access_nonexistent_denied() {
    let (storage, _dir) = make_storage();
    let access = storage.check_universe_access(None, "does-not-exist");
    assert_eq!(access, crate::models::UniverseAccess::Denied);
}

/// Subscribe/unsubscribe flow: subscriptions table is correctly updated.
#[test]
fn test_subscribe_unsubscribe_flow() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "pub3".into(),
                name: "Public3".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "pub3", "public-subscribable");

    // Not subscribed yet.
    assert!(!storage.is_subscribed("user-a", "pub3"));

    // Subscribe.
    storage.subscribe_universe("user-a", "pub3").unwrap();
    assert!(storage.is_subscribed("user-a", "pub3"));

    // Appears in user's universe list.
    let universes = storage.list_universes_for_user("user-a");
    assert!(
        universes.iter().any(|u| u.key == "pub3"),
        "subscribed universe must appear in user list"
    );

    // Unsubscribe.
    storage.unsubscribe_universe("user-a", "pub3").unwrap();
    assert!(!storage.is_subscribed("user-a", "pub3"));

    // No longer in user's universe list.
    let universes_after = storage.list_universes_for_user("user-a");
    assert!(
        !universes_after.iter().any(|u| u.key == "pub3"),
        "unsubscribed universe must not appear in user list"
    );
}

/// CO-314: list_subscribed_universes returns subscribed rows (not silently empty).
/// Regression test for the param-count bug where the query referenced `?1`
/// three times but passed three positional params, which rusqlite rejected
/// as "Wrong number of parameters passed to query. Got 2, needed 1" and
/// dropped every row. The SPA's Subscribe button looked broken because
/// /me/universes returned subscribed:[] even with rows in the DB.
#[test]
fn test_list_subscribed_universes_returns_rows() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "pub-sub".into(),
                name: "Public Sub".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "pub-sub", "public-subscribable");

    storage.subscribe_universe("user-a", "pub-sub").unwrap();

    let subscribed = storage.list_subscribed_universes("user-a");
    assert_eq!(
        subscribed.len(),
        1,
        "list_subscribed_universes must return subscribed rows"
    );
    assert_eq!(subscribed[0].universe.key, "pub-sub");
}

/// Cannot subscribe to a private universe.
#[test]
fn test_cannot_subscribe_to_private_universe() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "private-u".into(),
                name: "Private".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();
    let result = storage.subscribe_universe("user-b", "private-u");
    assert!(
        result.is_err(),
        "subscribing to a private universe must fail"
    );
}

/// Search returns only public-subscribable universes matching the query.
#[test]
fn test_search_public_universes() {
    let (mut storage, _dir) = make_storage();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "co-dev".into(),
                name: "CO Development".into(),
                description: "The main dev board".into(),
            },
            "owner-1",
        )
        .unwrap();
    set_visibility(&storage, "co-dev", "public-subscribable");

    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "private-proj".into(),
                name: "Private Project".into(),
                description: String::new(),
            },
            "owner-1",
        )
        .unwrap();

    let results = storage.search_public_universes("dev");
    assert!(
        results.iter().any(|u| u.key == "co-dev"),
        "co-dev must appear in search results"
    );
    assert!(
        !results.iter().any(|u| u.key == "private-proj"),
        "private universe must not appear in search results"
    );
}
