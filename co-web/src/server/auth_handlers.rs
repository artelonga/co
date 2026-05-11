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

    // CO-184 reverse bridge — best-effort.
    {
        let storage = lock_storage(&state)?;
        if let Err(e) = storage.ensure_quilombo_user_for_co(&user_id) {
            tracing::warn!(
                "CO-184 ensure_quilombo_user_for_co failed for {user_id} (magic-link continues): {e}"
            );
        }
    }

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
        // CO-173: include per-universe metadata for the authenticated user.
        let universes = storage.list_universes_with_metadata_for_user(&user.id);
        // CO-198: include dm_policy so the frontend can pre-populate the privacy radio.
        let dm_policy: Option<String> = storage
            .conn()
            .query_row(
                "SELECT COALESCE(dm_policy,'shared-universe') FROM users WHERE id = ?1",
                rusqlite::params![user.id],
                |row| row.get(0),
            )
            .ok();
        return Ok(Json(MeResponse {
            user_id: user.id,
            email: user.email,
            display_name: user.display_name,
            tier: user.tier,
            universes,
            dm_policy,
        }));
    }

    if let Some(u) = crate::quilombo_storage::obter_usuario_por_id(storage.conn(), &user_id.0) {
        // CO-173: even when the principal is a quilombo user (no CO link yet),
        // surface the universes list — typically empty for unlinked quilombo
        // users, but a follow-up `linked_co_user_id` set will populate it.
        let universes = storage.list_universes_with_metadata_for_user(&u.id);
        return Ok(Json(MeResponse {
            user_id: u.id,
            email: String::new(),
            display_name: if u.nome.is_empty() {
                u.usuario.clone()
            } else {
                u.nome
            },
            tier: u.papel.to_string(),
            universes,
            dm_policy: None,
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

    // CO-184 reverse bridge — best-effort.
    {
        let storage = lock_storage(&state)?;
        if let Err(e) = storage.ensure_quilombo_user_for_co(&user.id) {
            tracing::warn!(
                "CO-184 ensure_quilombo_user_for_co failed for {} (password-login continues): {e}",
                user.id
            );
        }
    }

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

// --- Public signup (CO-175 / G3) ---

/// Request body for public signup. Username + password are required; email is
/// optional per `feedback_auth_email.md` — never harvested at signup, only
/// when the user opts into account recovery.
#[derive(serde::Deserialize)]
pub(super) struct SignupRequest {
    usuario: String,
    password: String,
    #[serde(default)]
    email: String,
}

/// CO-175 (G3): public signup quota — 100 new accounts per rolling 24h.
/// Caps abuse without further infra (CAPTCHA / IP throttling can ride later).
const SIGNUP_DAILY_CAP: i64 = 100;
const SIGNUP_WINDOW_SECONDS: i64 = 24 * 60 * 60;

/// POST /api/v1/auth/signup — create a new CO account.
///
/// Validation (rejects with 400 on failure):
/// - `usuario`: 3-30 chars, `[a-z0-9_-]`, lowercased before persist.
/// - `password`: ≥8 chars (no upper bound enforced — Argon2id handles long inputs).
/// - `email`: optional, validated `local@host.tld` shape if present.
///
/// Conflicts (409): usuario or email already taken.
/// Rate-limit (429): more than 100 signups in the last 24h cluster-wide.
/// Success (200): writes `users` row with Argon2id hash + auto-promotes the
/// email (when supplied) as a verified recovery channel, and issues a session
/// cookie identical to `password-login`.
pub(super) async fn signup_handler(
    State(state): State<AppState>,
    Json(req): Json<SignupRequest>,
) -> Result<Response, AppError> {
    let usuario = req.usuario.trim().to_lowercase();
    let password = req.password;
    let email_opt = {
        let trimmed = req.email.trim().to_lowercase();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    };

    // --- validation ---
    if usuario.len() < 3 || usuario.len() > 30 {
        return Err(AppError::BadRequest(
            "Usuário deve ter entre 3 e 30 caracteres.".into(),
        ));
    }
    if !usuario
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(AppError::BadRequest(
            "Usuário aceita apenas letras minúsculas, dígitos, '-' e '_'.".into(),
        ));
    }
    if password.len() < 8 {
        return Err(AppError::BadRequest(
            "Senha deve ter pelo menos 8 caracteres.".into(),
        ));
    }
    if let Some(ref e) = email_opt {
        let parts: Vec<&str> = e.splitn(2, '@').collect();
        if parts.len() != 2 || parts[0].is_empty() || !parts[1].contains('.') {
            return Err(AppError::BadRequest("Email inválido.".into()));
        }
    }

    // --- rate limit (cluster-wide rolling window) ---
    {
        let storage = lock_storage(&state)?;
        let count = storage.count_users_created_since(SIGNUP_WINDOW_SECONDS);
        if count >= SIGNUP_DAILY_CAP {
            return Err(AppError::TooManyRequests(format!(
                "Limite diário de {SIGNUP_DAILY_CAP} contas atingido. Tente novamente em algumas horas."
            )));
        }
    }

    // --- Argon2id hash (CPU-bound, blocking) ---
    let password_hash = tokio::task::spawn_blocking(move || -> Result<String, AppError> {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| AppError::Internal(format!("argon2: {e}")))
    })
    .await
    .map_err(|_| AppError::Internal("Task join error".into()))??;

    // --- create user ---
    let user = {
        let mut storage = lock_storage(&state)?;
        match storage.create_user_with_password(&usuario, &password_hash, email_opt.as_deref()) {
            Ok(u) => u,
            Err(crate::storage::users::SignupError::UsuarioTaken) => {
                return Err(AppError::Conflict("Esse usuário já existe.".into()));
            }
            Err(crate::storage::users::SignupError::EmailTaken) => {
                return Err(AppError::Conflict("Esse email já está em uso.".into()));
            }
            Err(crate::storage::users::SignupError::Internal(msg)) => {
                return Err(AppError::Internal(msg));
            }
        }
    };

    // --- session cookie ---
    let jwt_secret = crate::auth::jwt_secret();
    let (token, expires_at) = sign_jwt(&user.id, &user.email, &user.tier, &jwt_secret)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let cookie =
        crate::auth::build_session_cookie(&token, state.config.cookie_domain.as_deref(), 604800);

    // CO-184 reverse bridge — best-effort.
    {
        let storage = lock_storage(&state)?;
        if let Err(e) = storage.ensure_quilombo_user_for_co(&user.id) {
            tracing::warn!(
                "CO-184 ensure_quilombo_user_for_co failed for {} (signup continues): {e}",
                user.id
            );
        }
    }

    // --- telemetry ---
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.signup",
            universe: String::new(),
            list: Some("public".to_string()),
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

/// CO-177: tells the UI whether Google OAuth is configured on this deploy.
/// Lets the login modal hide the "Continuar com Google" button when the
/// `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` env vars aren't set —
/// avoids a button that lands on 503.
pub(super) async fn google_status_handler() -> Json<serde_json::Value> {
    let configured =
        std::env::var("GOOGLE_CLIENT_ID").is_ok() && std::env::var("GOOGLE_CLIENT_SECRET").is_ok();
    Json(serde_json::json!({ "configured": configured }))
}
