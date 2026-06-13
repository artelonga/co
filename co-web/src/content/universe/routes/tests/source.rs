use super::support::*;
use rusqlite::params;

// --- CO-330: migration backfill + source endpoint tests ---

/// CO-330: migration v51 adds the 3 columns and update_universe_source works.
#[test]
fn test_co330_migration_columns_and_source_update() {
    use rusqlite::OptionalExtension;

    let (mut storage, _dir) = make_storage();

    // 1. Verify the 3 columns were added by v51.
    let col_exists = |col: &str| -> bool {
        storage
            .conn()
            .query_row(
                "SELECT 1 FROM pragma_table_info('universes') WHERE name = ?1",
                [col],
                |_| Ok(true),
            )
            .optional()
            .unwrap()
            .unwrap_or(false)
    };
    assert!(
        col_exists("local_repo_path"),
        "local_repo_path column missing"
    );
    assert!(
        col_exists("content_subdirs"),
        "content_subdirs column missing"
    );
    assert!(
        col_exists("anon_published_only"),
        "anon_published_only column missing"
    );

    // 2. Verify update_universe_source works on the default universe.
    storage
        .update_universe_source(
            "default",
            Some("~/projects/test"),
            Some(r#"["docs","content"]"#),
            Some(true),
            None,
            None,
        )
        .expect("update_universe_source must succeed");

    let universe = storage
        .get_universe("default")
        .expect("default universe must exist");
    assert!(
        universe.anon_published_only,
        "anon_published_only must be true after update"
    );

    // 3. Verify anon_published_only defaults to false for freshly created universes.
    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT OR IGNORE INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES ('usr_co330','co330@example.com','CO330','player',?1,'co330')",
            rusqlite::params![now],
        )
        .unwrap();
    storage
        .create_universe(
            crate::models::CreateUniverse {
                key: "co330-test".to_string(),
                name: "CO330 Test".to_string(),
                description: String::new(),
            },
            "usr_co330",
        )
        .expect("create_universe must succeed");
    let new_uni = storage
        .get_universe("co330-test")
        .expect("co330-test must exist");
    assert!(
        !new_uni.anon_published_only,
        "new universe must default to anon_published_only=false"
    );
}

/// CO-330: PATCH /api/v1/universes/:slug/source — owner succeeds, non-owner gets 403.
#[tokio::test]
async fn test_patch_universe_source_owner_and_403() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();

    let now = chrono::Utc::now().to_rfc3339();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES ('usr_owner','owner@example.com','Owner','player',?1,'owner')",
            params![now],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "INSERT INTO users (id, email, display_name, tier, created_at, usuario) \
             VALUES ('usr_other','other@example.com','Other','player',?1,'other')",
            params![now],
        )
        .unwrap();

    let (router, _tmp) = make_universe_router(storage, dir.path());
    let (owner_token, _) = crate::auth::sign_jwt(
        "usr_owner",
        "owner@example.com",
        "player",
        "test-jwt-secret",
    )
    .unwrap();
    let (other_token, _) = crate::auth::sign_jwt(
        "usr_other",
        "other@example.com",
        "player",
        "test-jwt-secret",
    )
    .unwrap();

    // Create a universe owned by usr_owner.
    let create_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {owner_token}"))
                .body(Body::from(
                    r#"{"key":"src-test","name":"Source Test","description":""}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), axum::http::StatusCode::CREATED);

    let patch_body = r#"{"local_repo_path":"~/projects/src-test","content_subdirs":["docs"],"anon_published_only":true}"#;

    // Owner PATCH → 200.
    let patch_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/universes/src-test/source")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {owner_token}"))
                .body(Body::from(patch_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(patch_resp.status(), axum::http::StatusCode::OK);
    let body = body_bytes(patch_resp).await;
    let resp: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(resp["key"], "src-test");
    assert_eq!(resp["anon_published_only"], true);
    assert_eq!(resp["local_repo_path"], "~/projects/src-test");

    // Non-owner PATCH → 403.
    let forbidden_resp = router
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/universes/src-test/source")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {other_token}"))
                .body(Body::from(patch_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden_resp.status(), axum::http::StatusCode::FORBIDDEN);
}
