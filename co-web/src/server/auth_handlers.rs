use super::*;

// --- Experiment Handlers ---

pub(super) async fn get_variant(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Json<VariantResponse> {
    let variant = extract_variant(&headers, &state.config);
    Json(VariantResponse { variant })
}

pub(super) async fn switch_variant(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<SwitchVariant>,
) -> Result<Response, AppError> {
    let participant_id =
        extract_participant(&headers).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let variant = body.variant.clone();
    if !matches!(
        variant.as_str(),
        "a" | "b" | "c" | "d" | "e" | "f" | "g" | "h"
    ) {
        return Err(AppError::BadRequest("Invalid variant".into()));
    }

    {
        let mut experiment = lock_experiment(&state)?;
        experiment.switch_variant(&participant_id, &variant);
    }

    let cookie = format!(
        "co_variant={}; Path=/; SameSite=Lax; HttpOnly; Max-Age=31536000",
        variant
    );

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(VariantResponse { variant }),
    )
        .into_response())
}

pub(super) async fn submit_feedback(
    headers: HeaderMap,
    State(state): State<AppState>,
    Json(body): Json<SubmitFeedback>,
) -> Result<impl IntoResponse, AppError> {
    let participant_id =
        extract_participant(&headers).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let variant = extract_variant(&headers, &state.config);

    let mut experiment = lock_experiment(&state)?;
    let entry = experiment.submit_feedback(&participant_id, &variant, body);

    Ok((StatusCode::CREATED, Json(entry)))
}

pub(super) async fn get_summary(
    State(state): State<AppState>,
) -> Result<Json<ExperimentSummary>, AppError> {
    let experiment = lock_experiment(&state)?;
    Ok(Json(experiment.get_summary()))
}

// --- Auth Handlers ---

pub(super) async fn login_handler(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let email = body.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(AppError::BadRequest("Email is required".into()));
    }

    // Rate limit check.
    {
        let auth = lock_auth(&state)?;
        if !auth.check_rate_limit(&email)? {
            return Err(AppError::TooManyRequests(
                "Too many code requests. Please wait before requesting another.".into(),
            ));
        }
        auth.record_request(&email)?;
    }

    // Look up user — new emails auto-register on verify, so always send code.
    let user_id = {
        let storage = lock_storage(&state)?;
        storage.get_user_by_email(&email).map(|u| u.id)
    };

    let code = generate_code();
    let entry = new_code_entry(user_id, code.clone());

    {
        let auth = lock_auth(&state)?;
        auth.store_code(&email, &entry)?;
    }

    let subject = "Seu código de acesso";
    let body_text =
        format!("Seu código de verificação é: {code}\n\nEste código expira em 5 minutos.");
    if let Err(e) = state.mail.send(&email, subject, &body_text) {
        tracing::warn!("Failed to send verification email to {email}: {e}");
    }

    Ok(Json(LoginResponse {
        message: "If registered, a code has been sent to your email".into(),
    }))
}

pub(super) async fn verify_handler(
    State(state): State<AppState>,
    Json(body): Json<VerifyRequest>,
) -> Result<Response, AppError> {
    let email = body.email.trim().to_lowercase();
    let code = body.code.trim().to_string();

    let entry = {
        let auth = lock_auth(&state)?;
        auth.get_code(&email)?
    };

    let entry = match entry {
        None => return Err(AppError::Gone("Code not found or already used".into())),
        Some(e) => e,
    };

    // Check expiry.
    if Utc::now() > entry.expires_at {
        let auth = lock_auth(&state)?;
        auth.delete_code(&email)?;
        return Err(AppError::Gone("Code has expired".into()));
    }

    if entry.code != code {
        let new_attempts = entry.attempts.saturating_sub(1);

        if new_attempts == 0 {
            let auth = lock_auth(&state)?;
            auth.delete_code(&email)?;
            let body = serde_json::json!({ "error": "Code expired, request a new one" });
            return Ok((StatusCode::UNAUTHORIZED, Json(body)).into_response());
        }

        // Update attempts.
        let updated = crate::auth::VerifyCodeEntry {
            attempts: new_attempts,
            ..entry
        };
        {
            let auth = lock_auth(&state)?;
            auth.store_code(&email, &updated)?;
        }

        let body = serde_json::json!({ "remaining_attempts": new_attempts });
        return Ok((StatusCode::UNAUTHORIZED, Json(body)).into_response());
    }

    // Code matches — resolve or create user.
    let (user_id, display_name, tier) = match entry.user_id {
        Some(ref id) => {
            let storage = lock_storage(&state)?;
            let u = storage
                .get_user_by_id(id)
                .unwrap_or_else(|| crate::models::User {
                    id: id.clone(),
                    email: email.clone(),
                    display_name: String::new(),
                    tier: "player".to_string(),
                    created_at: Utc::now(),
                    usuario: None,
                });
            (id.clone(), u.display_name, u.tier)
        }
        None => {
            // First-time user — auto-register.
            let display_name = email.split('@').next().unwrap_or("user").to_string();
            let user = {
                let mut storage = lock_storage(&state)?;
                storage
                    .create_user(&email, &display_name)
                    .map_err(|e| AppError::Internal(e.to_string()))?
            };
            tracing::info!("Auto-registered new user: {} <{}>", user.id, email);
            (user.id, user.display_name, user.tier)
        }
    };

    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret".to_string());
    let (token, expires_at) = sign_jwt(&user_id, &email, &tier, &jwt_secret)?;

    // Delete used code.
    {
        let auth = lock_auth(&state)?;
        auth.delete_code(&email)?;
    }

    let cookie =
        crate::auth::build_session_cookie(&token, state.config.cookie_domain.as_deref(), 604800);

    // CO-156: emit auth.login telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.login",
            universe: String::new(),
            list: Some("magic-link".to_string()),
            key: Some(user_id.clone()),
            actor: Some(user_id.clone()),
            session_id: None,
            extra: None,
        },
    );

    let response_body = VerifyResponse {
        user_id,
        email,
        display_name,
        expires_at,
    };

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response_body),
    )
        .into_response())
}

// --- Auth: Me & Logout ---

pub(super) async fn me_handler(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
) -> Result<Json<MeResponse>, AppError> {
    let storage = lock_storage(&state)?;

    // Check board users table first, then fall back to quilombo users.
    if let Some(user) = storage.get_user_by_id(&user_id.0) {
        return Ok(Json(MeResponse {
            user_id: user.id,
            email: user.email,
            display_name: user.display_name,
            tier: user.tier,
        }));
    }

    if let Some(u) = crate::quilombo_storage::obter_usuario_por_id(storage.conn(), &user_id.0) {
        return Ok(Json(MeResponse {
            user_id: u.id,
            email: String::new(),
            display_name: if u.nome.is_empty() {
                u.usuario.clone()
            } else {
                u.nome
            },
            tier: u.papel.to_string(),
        }));
    }

    Err(AppError::NotFound("User not found".into()))
}

/// GET /api/v1/auth/stats — user statistics (universes count, sizes, articles, etc.)
pub(super) async fn user_stats_handler(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
) -> Result<Json<serde_json::Value>, AppError> {
    let storage = lock_storage(&state)?;
    let universes = storage.list_universes_for_user(&user_id.0);

    let mut stats = Vec::new();
    for u in &universes {
        let entry_count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1",
                rusqlite::params![u.key],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let page_count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1 AND entry_type = 'page'",
                rusqlite::params![u.key],
                |row| row.get(0),
            )
            .unwrap_or(0);

        let task_count: i64 = storage
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE universe_key = ?1 AND entry_type = 'task'",
                rusqlite::params![u.key],
                |row| row.get(0),
            )
            .unwrap_or(0);

        stats.push(serde_json::json!({
            "key": u.key,
            "name": u.name,
            "entries": entry_count,
            "pages": page_count,
            "tasks": task_count,
            "content_count": u.content_count,
            "is_public": u.is_public,
        }));
    }

    Ok(Json(serde_json::json!({
        "user_id": user_id.0,
        "universes": stats,
        "total_universes": universes.len(),
        "total_entries": stats.iter().map(|s| s["entries"].as_i64().unwrap_or(0)).sum::<i64>(),
    })))
}

pub(super) async fn logout_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    // CO-156: emit auth.logout telemetry (best-effort, before cookie is cleared)
    let session_id = crate::telemetry::extract_session_id(&headers);
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.logout",
            universe: String::new(),
            list: None,
            key: None,
            actor: crate::auth::resolve_user_id(&state, &headers),
            session_id,
            extra: None,
        },
    );

    let clear_cookie = "session=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0";
    (
        StatusCode::OK,
        [(header::SET_COOKIE, clear_cookie)],
        Json(serde_json::json!({ "message": "Logged out" })),
    )
        .into_response()
}

// --- Password-based login (CO-85) ---

/// Request body for password login.
#[derive(serde::Deserialize)]
pub(super) struct PasswordLoginRequest {
    #[serde(default)]
    email: String,
    #[serde(default)]
    usuario: String,
    password: String,
}

/// POST /api/v1/auth/password-login — Argon2id password login, any environment.
///
/// Accepts `email` or `usuario` field (CO-165: username+email decoupling).
/// Succeeds when the user record has a non-NULL `password_hash` set.
/// Returns 401 for unknown email/usuario, wrong password, or missing hash — all
/// indistinguishable to callers to prevent user enumeration.
pub(super) async fn password_login_handler(
    State(state): State<AppState>,
    Json(req): Json<PasswordLoginRequest>,
) -> Result<Response, AppError> {
    let (user, hash_opt) = {
        let storage = lock_storage(&state)?;
        if !req.email.is_empty() {
            let email = req.email.trim().to_lowercase();
            storage
                .get_user_by_email_with_hash(&email)
                .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?
        } else if !req.usuario.is_empty() {
            let usuario = req.usuario.trim().to_lowercase();
            let user = storage
                .get_user_by_usuario(&usuario)
                .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?;
            storage
                .get_user_by_id_with_hash(&user.id)
                .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?
        } else {
            return Err(AppError::Unauthorized("Invalid credentials".into()));
        }
    };

    let hash = hash_opt.ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?;

    // Verify password with Argon2id (blocking — CPU-intensive).
    let password = req.password.clone();
    let hash_clone = hash.clone();
    tokio::task::spawn_blocking(move || {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHash, PasswordVerifier};
        let parsed =
            PasswordHash::new(&hash_clone).map_err(|_| AppError::Internal("Bad hash".into()))?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| AppError::Unauthorized("Invalid credentials".into()))
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    let jwt_secret = crate::auth::jwt_secret();
    let (token, expires_at) = sign_jwt(&user.id, &user.email, &user.tier, &jwt_secret)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let cookie =
        crate::auth::build_session_cookie(&token, state.config.cookie_domain.as_deref(), 604800);

    // CO-156: emit auth.login telemetry
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.login",
            universe: String::new(),
            list: Some("password".to_string()),
            key: Some(user.id.clone()),
            actor: Some(user.id.clone()),
            session_id: None,
            extra: None,
        },
    );

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(VerifyResponse {
            user_id: user.id,
            email: user.email,
            display_name: user.display_name,
            expires_at,
        }),
    )
        .into_response())
}

/// POST /api/v1/auth/uat-login — compat alias for UAT scripts and CLAUDE.md docs.
///
/// Delegates to `password_login_handler` when `CO_ENV=uat`; returns 404 in
/// production so the endpoint existence is not revealed to non-UAT deployments.
pub(super) async fn uat_login_handler(
    State(state): State<AppState>,
    Json(req): Json<PasswordLoginRequest>,
) -> Result<Response, AppError> {
    if !state.config.is_uat() {
        return Err(AppError::NotFound("Not found".into()));
    }
    password_login_handler(State(state), Json(req)).await
}
