use super::*;

pub fn build_router(state: AppState, plugin_routes: Option<Router<AppState>>) -> Router {
    let auth_api = Router::new()
        .route("/v1/auth/login", post(login_handler))
        .route("/v1/auth/verify", post(verify_handler))
        .route(
            "/v1/auth/me",
            get(me_handler).layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::auth::require_auth,
            )),
        )
        .route(
            "/v1/auth/stats",
            get(user_stats_handler).layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::auth::require_auth,
            )),
        )
        .route("/v1/auth/logout", post(logout_handler))
        // CO-85: password-based login (any env, user must have password_hash set)
        .route("/v1/auth/password-login", post(password_login_handler))
        .route("/v1/auth/signup", post(signup_handler))
        // CO-177: Google OAuth. Status hides the button when not configured.
        .route("/v1/auth/google/status", get(google_status_handler))
        .nest("/v1/auth", crate::oauth_google::router())
        // CO-415: GitHub OAuth. Status hides the button when not configured.
        .route("/v1/auth/github/status", get(github_status_handler))
        .nest("/v1/auth", crate::oauth_github::router())
        // CO-44: compat alias — returns 404 in prod
        .route("/v1/auth/uat-login", post(uat_login_handler))
        // CO-303: login-options tells the SPA which auth tabs to render
        .route("/v1/auth/login-options", get(login_options_handler));

    let board_public = Router::new()
        .route("/projects/{key}", get(get_project))
        .route("/projects/{key}/tasks", get(list_tasks))
        .route("/projects/{key}/tasks/{id}", get(get_task))
        .route("/projects/{key}/tasks/{id}/comments", get(list_comments))
        .route("/projects/{key}/activity", get(list_activity))
        .route("/projects/{key}/dashboard", get(get_dashboard))
        .route("/health", get(health_check))
        .route("/health/deep", get(health_check_deep));

    let board_protected = Router::new()
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{key}", delete(delete_project))
        .route("/projects/{key}/tasks", post(create_task))
        .route(
            "/projects/{key}/tasks/{id}",
            put(update_task).delete(delete_task),
        )
        .route("/projects/{key}/tasks/{id}/comments", post(create_comment))
        .route("/projects/{key}/tasks/bulk-update", post(bulk_update_tasks))
        .route("/projects/{key}/tasks/bulk-delete", post(bulk_delete_tasks))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_auth,
        ));

    let experiment_api = Router::new()
        .route("/experiment/variant", get(get_variant).post(switch_variant))
        .route("/experiment/feedback", post(submit_feedback))
        .route("/experiment/summary", get(get_summary));

    use crate::game_routes;

    let game_public = Router::new()
        .route("/v1/health", get(game_routes::health))
        .route("/v1/plugins", get(game_routes::list_plugins))
        .route("/v1/auth/register", post(game_routes::register))
        .route("/v1/auth/legacy-login", post(game_routes::legacy_login))
        .route(
            "/v1/games/{game_name}/leaderboard",
            get(game_routes::get_leaderboard),
        )
        .route(
            "/v1/games/leaderboard/global",
            get(game_routes::get_global_leaderboard),
        )
        .route("/v1/games/recent", get(game_routes::get_recent_activity))
        .route(
            "/v1/players/{username}",
            get(game_routes::get_player_profile),
        );

    let game_protected = Router::new()
        .route("/v1/profile", get(game_routes::get_profile))
        .route("/v1/wallet", get(game_routes::get_wallet))
        .route(
            "/v1/games/{game_name}/result",
            post(game_routes::record_game_result),
        )
        .route(
            "/v1/games/{game_name}/stats",
            get(game_routes::get_game_stats),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_auth,
        ));

    // CO-205: mirror_request() echoes the caller's Origin so `credentials: 'include'`
    // works for any safelisted origin (artelonga.com.br → co.artelonga.com.br).
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
            HeaderName::from_static("target-type"),
            HeaderName::from_static("target"),
            HeaderName::from_static("operation"),
            HeaderName::from_static("x-admin-override-quota"),
        ]);

    let quilombo_api = crate::quilombo_routes::router(state.clone());

    // CO-435: admin auth now goes through the `AdminAuthProvider` trait. We
    // inject a single shared `Arc<dyn AdminAuthProvider>` (the GitHub default)
    // as an Extension; the `require_github_admin` middleware depends on the
    // trait, not on GitHub. Swapping in SAML/OIDC = build a different provider
    // here. The provider owns the verified-token cache, so all admin routers
    // share it. See `crate::infra::admin_auth`.
    let github_token_cache = crate::github_auth::new_token_cache();
    let allowed_admins = state.core.config.gestao_github_admins.clone();
    let admin_auth: Arc<dyn crate::infra::admin_auth::AdminAuthProvider> = Arc::new(
        crate::github_auth::GitHubAdminAuthProvider::new(github_token_cache, allowed_admins),
    );

    let gestao_api = crate::gestao_routes::router().layer(axum::Extension(admin_auth.clone()));

    let telemetry_admin =
        crate::telemetry::admin_router().layer(axum::Extension(admin_auth.clone()));

    let gestao_oauth_api =
        crate::oidc_routes::gestao_oauth_router().layer(axum::Extension(admin_auth.clone()));

    let ab_admin = crate::ab_routes::admin_router().layer(axum::Extension(admin_auth.clone()));

    let webhook_admin = crate::webhook_routes::router().layer(axum::Extension(admin_auth.clone()));

    // CO-388: Security findings admin API (GitHub admin auth).
    let security_admin = crate::security::routes::router().layer(axum::Extension(admin_auth));

    let telemetry_public = crate::telemetry::router();
    let universe_api = crate::universe_routes::router(state.clone());

    let universe_invitation_api = crate::invitation_routes::universe_invitation_router().layer(
        axum::middleware::from_fn_with_state(state.clone(), crate::auth::require_auth),
    );
    let invitation_api = crate::invitation_routes::invitation_router(state.clone());

    let chat_api = crate::chat::chat_router().layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::auth::require_auth,
    ));

    let dm_api = crate::dm_routes::dm_router().layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::auth::require_auth,
    ));

    let themes_api = crate::universe_routes::themes_router();
    let vault_api = crate::vault_routes::vault_router();
    let token_api = crate::vault_routes::token_router(state.clone());
    let entry_api = crate::entry_routes::router();
    let relation_api = crate::relation_routes::router();
    let asset_api = crate::asset_routes::asset_router();
    let reference_api = crate::reference_routes::reference_router();
    // CO-335: graph endpoint
    let graph_api = crate::graph_routes::router();
    // CO-345: graph views (publishable saved views)
    let graph_view_me_api = crate::graph_view_routes::me_router().layer(
        axum::middleware::from_fn_with_state(state.clone(), crate::auth::require_auth),
    );
    let graph_view_public_api = crate::graph_view_routes::public_router();

    // CO-161: single visibility + writer gate. Every route nested here inherits
    // the access-control check — no per-handler boilerplate needed.
    let universe_content_api = Router::new()
        .merge(vault_api)
        .merge(entry_api)
        // CO-416: content translation (pt↔en) — owner-gated twin generation.
        .merge(crate::translate_routes::router())
        .merge(relation_api)
        .merge(asset_api)
        .merge(reference_api)
        .merge(graph_api)
        .merge(crate::state_routes::router())
        .merge(crate::branch_routes::router())
        .merge(crate::proposal_routes::router())
        .merge(crate::universe_routes::universe_actions_router())
        // CO-95: op log, replay, diff, promote, revert, cherry-pick
        .merge(crate::op_log_routes::router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::universe_writer_gate,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::universe_visibility_gate,
        ));

    // CO-354: suggest/review pipeline. The `/suggest` endpoint accepts
    // anonymous submissions, so this router deliberately sits OUTSIDE the
    // `universe_writer_gate` (which would 401 anon POSTs). The visibility gate
    // still applies — private universes require auth to reach — and the
    // owner-only review actions enforce ownership in-handler via `OwnerOf`.
    let suggest_review_api = crate::review_routes::router().layer(
        axum::middleware::from_fn_with_state(state.clone(), crate::auth::universe_visibility_gate),
    );

    let contact_api = crate::contact_routes::contact_router();

    // CO-398: delivery pipeline — GitHub inbound webhook + lead-time metrics.
    let delivery_api = crate::delivery_routes::router();
    let delivery_universe_api = crate::delivery_routes::universe_router();

    let log_drain_api = crate::log_drain_routes::router();
    let uat_api = crate::uat_routes::router(state.clone());
    let dev_board_api = crate::dev_board::router();
    let cache_api = Router::new().route("/stats", get(cache_stats_handler));

    // CO-385: conflict resolution REST API (auth enforced inside router).
    let sync_conflict_api = crate::sync::router(state.clone());

    // CRDT WebSocket — no body limit, auth done inside the handler.
    let ws_route = Router::new().route("/ws/doc/{slug}/{doc_id}", get(crate::ws::ws_handler));
    let sync_ws_route =
        Router::new().route("/api/v1/sync/ws", get(crate::sync_ws::sync_ws_handler));
    // Chat WebSocket — auth done inside handler.
    let chat_ws_route = Router::new().route(
        "/api/v1/universes/{slug}/chat/rooms/{room_slug}/ws",
        get(crate::chat::chat_ws_handler),
    );
    // CO-329: Analytics real-time stream — auth done inside handler.
    let analytics_ws_route = crate::analytics_routes::router();
    // CO-380: Universal EDA event stream — auth/visibility enforced in handler.
    let eda_events_ws_route = Router::new()
        .route(
            "/api/v1/events",
            get(crate::eda::events_ws::events_ws_handler),
        )
        .with_state(state.clone());
    // CO-384: Federated bridge — trust-list enforced in handler (CO_BRIDGE_TRUSTED_SOURCES).
    let eda_bridge_ws_route = Router::new()
        .route(
            "/api/v1/events/bridge",
            get(crate::eda::bridge::bridge_ws_handler),
        )
        .with_state(state.clone());

    // CO-397: robots.txt + sitemap.xml (must be before the /{slug} catch-all).
    let crawl_routes = crate::server::crawl_routes::router();

    // All literal routes are registered before `/{slug}` so axum's matcher
    // prefers them over the param capture.
    let co_routes = Router::new()
        .route("/", get(serve_co_index))
        // CO-329: real-time analytics dashboard (auth-gated, noindex)
        .route(
            "/analytics",
            get(crate::analytics_routes::serve_analytics_page),
        )
        // CO-260: standalone cross-version changelog viewer page
        .route(
            "/changelog",
            get(crate::changelog_routes::serve_changelog_page),
        )
        .route("/admin", get(crate::admin_routes::serve_admin_page))
        // CO-361: gestao SPA — GitHub PAT auth is handled client-side
        .route("/gestao", get(crate::gestao_routes::serve_gestao_page))
        .route(
            "/admin/deployments",
            get(crate::deployment_dashboard::serve_deployments_page),
        )
        .route("/repl", get(crate::repl_routes::serve_repl_page))
        .route(
            "/storage",
            get(crate::storage_dashboard::serve_storage_page),
        )
        .route(
            "/admin/leads.html",
            get(crate::lead_routes::serve_leads_page),
        )
        .route(
            "/co/telemetria",
            get(crate::telemetry::serve_admin_dashboard),
        )
        .route("/settings/sync", get(serve_sync_settings))
        .route("/yggdrasil/{game}", get(serve_co_index))
        .route("/notifications", get(serve_co_index))
        // CO-172: serve_recover validates return_to before serving the SPA — closes
        // the open-redirect phishing vector (co.artelonga.com.br/recover?return_to=evil).
        .route("/recover", get(serve_recover))
        // CO-206: issues a short-lived ES256 handover token for cross-apex SSO.
        .route(
            "/auth/co-handover",
            get(co_handover_handler).layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::auth::require_auth,
            )),
        )
        .route("/invitations/{token}", get(serve_co_index))
        // CO-381: live timeline — /agora (pt-BR) and /live (en) are public.
        .route("/agora", get(crate::live_routes::serve_agora_page))
        .route("/live", get(crate::live_routes::serve_live_page))
        // CO-170: friendly PT/EN aliases for the timeline composite view.
        .route(
            "/linhadotempo",
            get(|| async {
                axum::response::Redirect::temporary(
                    "/shared/timeline.html?u=tempo,universo,humanity",
                )
            }),
        )
        .route(
            "/timeline",
            get(|| async {
                axum::response::Redirect::temporary(
                    "/shared/timeline.html?u=tempo,universo,humanity",
                )
            }),
        )
        // 2.7.20: `/co/{slug}` redirects removed — `co` is now a universe slug.
        .route("/{slug}/assets", get(serve_assets_page))
        .route("/{slug}", get(serve_co_index))
        // 2.12.2 hotfix: `/{slug}/` (trailing slash, empty subpath) doesn't match
        // the `{*subpath}` wildcard below, so add an explicit trailing-slash route
        // to serve the SPA shell. Without this, `/entrar/`, `/sobre/`, `/termos/` 404.
        .route("/{slug}/", get(serve_co_index))
        // CO-354: suggest form + owner review queue — before the {*subpath} wildcard.
        .route("/{slug}/suggest", get(serve_suggest_page))
        .route("/{slug}/review", get(serve_review_page))
        // CO-335: universe graph viewer — must be before the {*subpath} wildcard.
        .route("/{slug}/graph", get(serve_graph_page))
        // CO-345: saved graph view viewer — must be before the {*subpath} wildcard.
        .route("/graph-views/{slug}", get(serve_graph_page))
        // CO-352: sala (workspace canvas) routes — literal paths before wildcard.
        .route("/u/{universe}/sala", get(serve_sala_page))
        .route("/u/{universe}/sala/{workspace_slug}", get(serve_sala_page))
        .route("/sala/{share_token}", get(serve_sala_page))
        // CO-372: sprint calendar SPA — must be before the {*subpath} wildcard.
        .route(
            "/scrum/calendar",
            get(crate::scrum::calendar::serve_calendar_page),
        )
        // CO-144: deeper SPA paths — must come AFTER the more specific routes.
        // CO-232: serve_deep_link validates entry existence and returns 404 when absent.
        .route("/{slug}/{*subpath}", get(serve_deep_link));

    let oauth_api = crate::oidc_routes::oauth_router(state.clone());
    let analytics_public_api = crate::analytics_public::router();
    let admin_dashboard_api = crate::admin_routes::api_router();
    let leads_public_api = crate::lead_routes::public_router();
    let leads_admin_api = crate::lead_routes::admin_router();

    // CO-211: OpenAPI spec + Swagger UI — no auth, no body limit override needed.
    let openapi_api = crate::openapi_routes::router();

    let mut router =
        Router::new()
            .merge(ws_route)
            .merge(sync_ws_route)
            .merge(chat_ws_route)
            .merge(analytics_ws_route)
            .merge(eda_events_ws_route)
            .merge(eda_bridge_ws_route)
            // CO-397: robots.txt + sitemap.xml (merged before co_routes so
            // literal paths win over the /{slug} catch-all in co_routes).
            .merge(crawl_routes)
            .merge(co_routes)
            .nest("/api", openapi_api)
            .nest("/api", board_public)
            .nest("/api", board_protected)
            .nest("/api", auth_api)
            .nest("/api", experiment_api)
            .nest("/api", game_public)
            .nest("/api", game_protected)
            .nest("/api/v1/quilombo", quilombo_api)
            // CO-385: conflict resolution endpoints (auth enforced in sub-router).
            .nest("/api/v1", sync_conflict_api)
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::telemetry::telemetry_middleware,
            ))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::quilombo_telemetria::telemetry_middleware,
            ))
            .layer(axum::middleware::from_fn(
                crate::quilombo_telemetria::csrf_middleware,
            ))
            .layer(axum::middleware::from_fn(
                crate::quilombo_telemetria::canonical_host_middleware,
            ))
            // CO-323: detect *.artelonga.com.br subdomains and store universe key
            // in request extensions so SPA-serving handlers can inject the bootstrap
            // script that locks the client to a single-universe view.
            .layer(axum::middleware::from_fn(
                crate::server::subdomain_routing::subdomain_routing_middleware,
            ))
            // CO-360: unified dashboard endpoints (email admin auth) — registered
            // BEFORE gestao_api so /atividades, /resumo, /universes are found first.
            .nest("/api/v1/gestao", crate::resumo_routes::api_router())
            .nest("/api/v1/gestao", gestao_api)
            .nest("/api/v1/gestao", webhook_admin)
            // CO-388: Security findings API (admin-gated).
            .nest("/api/v1/gestao/security", security_admin)
            // CO-142 Phase A: dev board moved to /api/v1/admin to un-shadow universe_api.
            .nest("/api/v1/admin", dev_board_api)
            .nest("/api/v1/universes", universe_api)
            .nest("/api/v1/universes", universe_invitation_api)
            .nest("/api/v1/invitations", invitation_api)
            .nest("/api/v1/universes", chat_api)
            .nest("/api/v1", dm_api)
            .nest(
                "/api/v1/me",
                crate::invitation_routes::me_invitations_router(state.clone()),
            )
            .nest(
                "/api/v1/me",
                crate::notification_routes::me_notifications_router(state.clone()),
            )
            .merge(crate::push_routes::vapid_router())
            .nest(
                "/api/v1/me",
                crate::push_routes::me_push_router(state.clone()),
            )
            .route(
                "/api/v1/me/universes",
                axum::routing::get(crate::universe_routes::me_universes_handler).layer(
                    axum::middleware::from_fn_with_state(state.clone(), crate::auth::require_auth),
                ),
            )
            // CO-326: public contact form — no auth required
            .nest("/api/v1/universes", contact_api)
            .nest("/api/v1/universes", universe_content_api)
            // CO-354: suggest/review pipeline — outside the writer gate so
            // anonymous visitors can submit suggestions.
            .nest("/api/v1/universes", suggest_review_api)
            // CO-244: read-only SQL query — auth required, but outside writer gate
            // since POST here is a query (not a mutation).
            .nest(
                "/api/v1/universes",
                crate::query_routes::router().layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::auth::require_auth,
                )),
            )
            // CO-355: workspace template registry — public read, optional auth on POST.
            .nest(
                "/api/v1/universes",
                crate::workspace_template_routes::router(),
            )
            // CO-387: calendar lens config (_calendar.yaml) — public read,
            // lens metadata only (no entry data).
            .nest("/api/v1/universes", crate::time::routes::router())
            // 2.7.23: inline proposals mounted OUTSIDE the writer gate — the handler
            // enforces its own auth + path constraints.
            .nest("/api/v1/universes", crate::proposal_routes::inline_router())
            .nest("/api/v1/me", crate::proposal_routes::inbox_router())
            // CO-352: workspace state — personal per-user, outside writer gate.
            .nest(
                "/api/v1/universes",
                crate::workspace_routes::public_router(),
            )
            .nest(
                "/api/v1/universes",
                crate::workspace_routes::authed_router().layer(
                    axum::middleware::from_fn_with_state(state.clone(), crate::auth::require_auth),
                ),
            )
            .nest("/api/v1", crate::workspace_routes::share_router())
            // CO-345: graph views — my views (auth-gated) + public/unlisted access
            .nest("/api/v1/me", graph_view_me_api)
            .nest("/api/v1", graph_view_public_api)
            // 1.75.0: blob CAS API — accepts JWT or long-lived API token.
            .nest(
                "/api/v1",
                crate::blob_routes::router().layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::auth::require_auth_with_token,
                )),
            )
            .nest("/api/v1/auth", token_api)
            .nest("/api/v1/themes", themes_api)
            .nest("/v1/log-drains/vercel", log_drain_api)
            .nest("/api/v1/uat", uat_api)
            .nest("/api/v1/telemetry", telemetry_public)
            .nest("/api/v1/admin", telemetry_admin)
            .nest("/api/v1/ab", ab_admin)
            .nest("/api/v1/cache", cache_api)
            .nest("/api/v1/admin", admin_dashboard_api)
            .nest("/api/v1/admin", crate::backup_routes::router())
            .nest("/api/v1/admin", crate::deployment_dashboard::router())
            .nest("/api/v1/admin", crate::storage_dashboard::router())
            .nest("/api/v1/me", crate::storage_dashboard::me_router())
            .nest(
                "/api/v1/universes",
                crate::storage_dashboard::universe_router(),
            )
            .nest("/api/v1/analytics/public", analytics_public_api)
            .nest("/api/v1", leads_public_api)
            .nest("/api/v1/admin", leads_admin_api)
            // CO-398: delivery pipeline webhook + metrics.
            .nest("/api/v1", delivery_api)
            .nest("/api/v1/universes", delivery_universe_api)
            .nest("/api/v1/processos", crate::processos::router())
            .nest("/oauth", oauth_api)
            .nest("/api/v1/gestao/oauth", gestao_oauth_api)
            .route(
                "/.well-known/openid-configuration",
                get(crate::oidc_routes::openid_configuration),
            )
            .route("/.well-known/jwks.json", get(crate::oidc_routes::jwks_json))
            // CO-260: cross-version changelog viewer API
            .nest("/api/v1", crate::changelog_routes::router())
            .nest(
                "/api/v1/admin",
                crate::changelog_routes::admin_router().layer(
                    axum::middleware::from_fn_with_state(state.clone(), crate::auth::require_auth),
                ),
            )
            // CO-275: agent session endpoints
            // GET is public (kanban lazy-loads); POST requires vault token or JWT.
            .nest("/api/v1", crate::agent_session_routes::router())
            .nest(
                "/api/v1",
                crate::agent_session_routes::authed_router().layer(
                    axum::middleware::from_fn_with_state(
                        state.clone(),
                        crate::auth::require_auth_with_token,
                    ),
                ),
            )
            // CO-426: usage ingestion + launcher registry + fleet query — all
            // require a vault token or JWT (same scheme as the agent-session write).
            .nest(
                "/api/v1",
                crate::usage_routes::authed_router().layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    crate::auth::require_auth_with_token,
                )),
            )
            .nest("/api/v1", crate::search_routes::router())
            .nest(
                "/api/v1/auth/recovery",
                crate::recovery_routes::recovery_router().layer(
                    axum::middleware::from_fn_with_state(state.clone(), crate::auth::require_auth),
                ),
            )
            .nest(
                "/api/v1/auth",
                crate::recovery_routes::forgot_password_router(state.clone()),
            )
            .nest(
                "/api/v1/auth",
                crate::onboarding_routes::onboarding_router(),
            );

    if let Some(plugin_router) = plugin_routes {
        router = router.nest("/api/v1/plugins", plugin_router);
    }

    // CO-372: Sprint calendar + ICS export (public, no auth).
    router = router.nest("/api/v1/scrum", crate::scrum::router());

    // CO-328: AI provider endpoints (Ollama + Claude Code hook).
    router = router.nest("/api/v1", crate::ai_routes::router(state.clone()));

    // CO-332: Public chat + deployment-status endpoints (non-Claude LLM, no auth).
    router = router.nest("/api/v1", crate::chat_routes::router(state.clone()));

    // CO-333: Feedback system (public submit, owner-only management).
    router = router.nest("/api/v1", crate::feedback_routes::router());

    // CO-367: Universal KB sync — POST /ingest (bearer token), GET /search, GET /recent.
    router = router.nest("/api/v1/kb", crate::kb_routes::router());

    router = router.nest("/api/v1/interactions", crate::interactions::router());

    // CO-329: analytics request tracking (runs inside rate-limit layer).
    router = router.layer(axum::middleware::from_fn(
        crate::observability::request_middleware,
    ));

    // CO-80: rate limiting applied after ALL routes so it covers every endpoint.
    router = router.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        crate::rate_limit::rate_limit_middleware,
    ));

    router
        .fallback(serve_variant_file)
        .with_state(state)
        .layer(DefaultBodyLimit::max(1_048_576))
        .layer(CompressionLayer::new())
        .layer(cors)
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        // CO-397: server version on all responses so LLM agents can identify
        // the CO release they are talking to.
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("x-co-server-version"),
            HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
        ))
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}
