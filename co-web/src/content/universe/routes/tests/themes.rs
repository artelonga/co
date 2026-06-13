use super::support::*;
use crate::models::UpdateUniverseFormConfig;

// --- CO-25: theme gating ---

/// Anonymous user (no auth header) sees 4 free palettes, no variants, no custom editor.
#[tokio::test]
async fn test_themes_available_anonymous() {
    let headers = axum::http::HeaderMap::new();
    let axum::Json(themes) = super::super::get_available_themes(headers).await;

    assert_eq!(
        themes.palettes,
        vec!["scholarly", "scholarly-dark", "relic", "relic-light"]
    );
    assert!(themes.variants.is_empty());
    assert!(themes.custom.is_none());
}

/// Real logged-in user sees Modern + 4 free palettes + 8 variants + custom editor.
#[tokio::test]
async fn test_themes_available_logged_in() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (token, _) =
        crate::auth::sign_jwt("usr_real", "user@example.com", "player", "test-jwt-secret").unwrap();

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    let axum::Json(themes) = super::super::get_available_themes(headers).await;

    assert_eq!(
        themes.palettes,
        vec!["", "scholarly", "scholarly-dark", "relic", "relic-light"]
    );
    assert_eq!(themes.variants.len(), 9);
    assert_eq!(themes.custom, Some(true));
}

/// Anon-tier user (cookie JWT with tier="anon") sees only free palettes.
#[tokio::test]
async fn test_themes_available_anon_cookie() {
    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (token, _) = crate::auth::sign_jwt("anon-abc123", "", "anon", "test-jwt-secret").unwrap();

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::COOKIE,
        format!("session={token}").parse().unwrap(),
    );
    let axum::Json(themes) = super::super::get_available_themes(headers).await;

    assert_eq!(
        themes.palettes,
        vec!["scholarly", "scholarly-dark", "relic", "relic-light"]
    );
    assert!(themes.variants.is_empty());
}

/// A premium theme (scholarly, relic) set by an owner persists even if the user logs out —
/// the storage layer always returns the stored preset regardless of auth.
#[test]
fn test_premium_theme_persists_after_owner_sets_it() {
    let (mut storage, _dir) = make_storage();

    // Owner sets a premium theme while logged in.
    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("relic".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

    // Reading config back (as if a new, unauthenticated visitor renders the universe)
    // must still return the premium theme — gating only applies to the switcher UI.
    let config = storage.get_universe_form_config("default").unwrap();
    assert_eq!(config.theme_preset, "relic");
}

#[tokio::test]
async fn test_theme_css_returns_ok() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let ct = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("text/css"), "Content-Type must be text/css");
    let body = body_bytes(response).await;
    assert!(body.contains(":root {"), "CSS must contain :root block");
    assert!(body.contains("--bg:"), "CSS must contain --bg token");
    assert!(
        body.contains("--accent:"),
        "CSS must contain --accent token"
    );
}

/// All required tokens are present in the generated CSS.
#[tokio::test]
async fn test_theme_css_all_required_tokens() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_bytes(response).await;

    for token in crate::theme_engine::tests::REQUIRED_TOKENS {
        assert!(
            body.contains(*token),
            "theme.css must contain token '{token}'"
        );
    }
}

/// Changing the theme changes the CSS output.
#[tokio::test]
async fn test_theme_css_changes_when_theme_changes() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (mut storage, dir) = make_storage();

    // Set theme to scholarly-dark
    storage
        .update_universe_form_config(
            "default",
            UpdateUniverseFormConfig {
                theme_preset: Some("scholarly-dark".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_bytes(response).await;

    // scholarly-dark --bg is #1c1610
    assert!(
        body.contains("#1c1610"),
        "scholarly-dark --bg must be #1c1610"
    );
    // Must NOT have scholarly-light --bg
    assert!(
        !body.contains("#FFF9ED"),
        "scholarly-dark must not contain scholarly-light --bg"
    );
}

/// GET /theme.css for a missing universe returns 404.
#[tokio::test]
async fn test_theme_css_404_for_missing_universe() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/no-such-universe/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

/// ETag is present and the same ETag triggers 304 Not Modified.
#[tokio::test]
async fn test_theme_css_etag_304() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    unsafe { std::env::set_var("JWT_SECRET", "test-jwt-secret") };
    let (storage, dir) = make_storage();
    let (router, _tmp) = make_universe_router(storage, dir.path());

    // First request: capture ETag.
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let etag = response
        .headers()
        .get(axum::http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .to_string();

    // Second request with If-None-Match: expect 304.
    let response2 = router
        .oneshot(
            Request::builder()
                .uri("/api/v1/universes/default/theme.css")
                .header(axum::http::header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response2.status(), axum::http::StatusCode::NOT_MODIFIED);
}
