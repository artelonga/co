use super::*;

// ---------------------------------------------------------------------------
// Typed response structs (CO-217)
// ---------------------------------------------------------------------------

/// Per-universe statistics row for `GET /api/v1/auth/stats`.
#[derive(Debug, serde::Serialize)]
pub(super) struct UniverseStat {
    pub key: String,
    pub name: String,
    pub entries: i64,
    pub pages: i64,
    pub tasks: i64,
    pub content_count: i64,
    pub is_public: bool,
}

/// Typed response for `GET /api/v1/auth/stats`.
#[derive(Debug, serde::Serialize)]
pub(super) struct UserStatsResponse {
    pub user_id: String,
    pub universes: Vec<UniverseStat>,
    pub total_universes: usize,
    pub total_entries: i64,
}

/// Typed response for `GET /api/v1/auth/google-status`.
#[derive(Debug, serde::Serialize)]
pub(super) struct GoogleStatusResponse {
    pub configured: bool,
}

/// CO-303: Available login methods for this deployment.
/// The SPA calls `GET /api/v1/auth/login-options` on modal mount and renders
/// only the supported paths (magic-code is always available; password and
/// Google are conditional).
#[derive(Debug, serde::Serialize)]
pub(super) struct LoginOptionsResponse {
    pub magic_code: bool,
    pub password: bool,
    pub google: bool,
}

// --- Experiment Handlers ---

pub(super) async fn get_variant(
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Json<VariantResponse> {
    let variant = extract_variant(&headers, &state.core.config);
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
    let variant = extract_variant(&headers, &state.core.config);

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
        let storage = lock_storage(&state);
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
    if let Err(e) = state.integrations.mail.send(&email, subject, &body_text) {
        tracing::warn!("Failed to send verification email to {email}: {e}");
    }

    // CO-303: surface code inline in non-prod envs so localhost devs can
    // complete login through the UI without email delivery.
    let dev_code = state.core.config.is_local_or_test().then(|| code.clone());

    Ok(Json(LoginResponse {
        message: "If registered, a code has been sent to your email".into(),
        dev_code,
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
            let storage = lock_storage(&state);
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
                let mut storage = lock_storage(&state);
                storage
                    .create_user(&email, &display_name)
                    .map_err(|e| AppError::Internal(e.to_string()))?
            };
            tracing::info!("Auto-registered new user: {} <{}>", user.id, email);
            (user.id, user.display_name, user.tier)
        }
    };

    let token = state
        .core
        .auth_provider
        .issue_token(
            crate::infra::auth::UserClaims {
                user_id: user_id.clone(),
                email: email.clone(),
                tier: tier.clone(),
                usuario: String::new(),
                papel: String::new(),
            },
            crate::infra::auth::DEFAULT_TOKEN_TTL,
        )
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let expires_at = Utc::now()
        + chrono::Duration::seconds(crate::infra::auth::DEFAULT_TOKEN_TTL.as_secs() as i64);

    // Delete used code.
    {
        let auth = lock_auth(&state)?;
        auth.delete_code(&email)?;
    }

    let cookie = crate::auth::build_session_cookie(
        token.as_str(),
        state.core.config.cookie_domain.as_deref(),
        604800,
    );

    // CO-184 reverse bridge — best-effort.
    {
        let storage = lock_storage(&state);
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

    // CO-361: audit log
    crate::atividade::log_atividade(
        state.clone(),
        crate::atividade::Atividade {
            acao: crate::atividade::Acao::Login,
            entidade: "user".into(),
            entidade_id: Some(user_id.clone()),
            before: None,
            after: None,
            tipo: crate::atividade::Tipo::Sucesso,
            user_id: Some(user_id.clone()),
            ip: None,
            user_agent: None,
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
    let storage = lock_storage(&state);

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
        // CO-370: include lead identity for funnel attribution.
        let (lead_id, lead_status, lead_source) = storage
            .conn()
            .query_row(
                "SELECT l.id, l.status, l.source \
                 FROM leads l \
                 INNER JOIN users u ON u.lead_id = l.id \
                 WHERE u.id = ?1 \
                 LIMIT 1",
                rusqlite::params![user.id],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap_or((None, None, None));
        return Ok(Json(MeResponse {
            user_id: user.id,
            email: user.email,
            display_name: user.display_name,
            tier: user.tier,
            universes,
            dm_policy,
            lead_id,
            lead_status,
            lead_source,
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
            lead_id: None,
            lead_status: None,
            lead_source: None,
        }));
    }

    Err(AppError::NotFound("User not found".into()))
}

/// GET /api/v1/auth/stats — user statistics (universes count, sizes, articles, etc.)
pub(super) async fn user_stats_handler(
    State(state): State<AppState>,
    user_id: crate::auth::UserId,
) -> Result<Json<UserStatsResponse>, AppError> {
    let storage = lock_storage(&state);
    let universes = storage.list_universes_for_user(&user_id.0);

    let mut stats: Vec<UniverseStat> = Vec::new();
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

        stats.push(UniverseStat {
            key: u.key.clone(),
            name: u.name.clone(),
            entries: entry_count,
            pages: page_count,
            tasks: task_count,
            content_count: u.content_count,
            is_public: u.is_public,
        });
    }

    let total_entries = stats.iter().map(|s| s.entries).sum();
    Ok(Json(UserStatsResponse {
        user_id: user_id.0,
        universes: stats,
        total_universes: universes.len(),
        total_entries,
    }))
}

pub(super) async fn logout_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    // CO-156: emit auth.logout telemetry (best-effort, before cookie is cleared)
    let session_id = crate::telemetry::extract_session_id(&headers);
    let actor = crate::auth::resolve_user_id(&state, &headers);
    crate::telemetry::emit_crud_event(
        &state,
        crate::telemetry::CrudEvent {
            kind: "auth.logout",
            universe: String::new(),
            list: None,
            key: None,
            actor: actor.clone(),
            session_id,
            extra: None,
        },
    );

    // CO-361: audit log
    crate::atividade::log_atividade(
        state.clone(),
        crate::atividade::Atividade {
            acao: crate::atividade::Acao::Logout,
            entidade: "user".into(),
            entidade_id: actor.clone(),
            before: None,
            after: None,
            tipo: crate::atividade::Tipo::Sucesso,
            user_id: actor,
            ip: None,
            user_agent: None,
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
        let storage = lock_storage(&state);
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

    let token = state
        .core
        .auth_provider
        .issue_token(
            crate::infra::auth::UserClaims {
                user_id: user.id.clone(),
                email: user.email.clone(),
                tier: user.tier.clone(),
                usuario: String::new(),
                papel: String::new(),
            },
            crate::infra::auth::DEFAULT_TOKEN_TTL,
        )
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let expires_at = Utc::now()
        + chrono::Duration::seconds(crate::infra::auth::DEFAULT_TOKEN_TTL.as_secs() as i64);

    let cookie = crate::auth::build_session_cookie(
        token.as_str(),
        state.core.config.cookie_domain.as_deref(),
        604800,
    );

    // CO-184 reverse bridge — best-effort.
    {
        let storage = lock_storage(&state);
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

    // CO-361: audit log (password field never in the diff — body is not captured)
    crate::atividade::log_atividade(
        state.clone(),
        crate::atividade::Atividade {
            acao: crate::atividade::Acao::Login,
            entidade: "user".into(),
            entidade_id: Some(user.id.clone()),
            before: None,
            after: None,
            tipo: crate::atividade::Tipo::Sucesso,
            user_id: Some(user.id.clone()),
            ip: None,
            user_agent: None,
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
    #[serde(default)]
    origin: Option<String>,
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
    let origin = crate::auth::sanitize_origin(req.origin);

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
        let storage = lock_storage(&state);
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
        let mut storage = lock_storage(&state);
        match storage.create_user_with_password(
            &usuario,
            &password_hash,
            email_opt.as_deref(),
            origin.as_deref(),
        ) {
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
    let token = state
        .core
        .auth_provider
        .issue_token(
            crate::infra::auth::UserClaims {
                user_id: user.id.clone(),
                email: user.email.clone(),
                tier: user.tier.clone(),
                usuario: String::new(),
                papel: String::new(),
            },
            crate::infra::auth::DEFAULT_TOKEN_TTL,
        )
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let expires_at = Utc::now()
        + chrono::Duration::seconds(crate::infra::auth::DEFAULT_TOKEN_TTL.as_secs() as i64);
    let cookie = crate::auth::build_session_cookie(
        token.as_str(),
        state.core.config.cookie_domain.as_deref(),
        604800,
    );

    // CO-184 reverse bridge — best-effort.
    {
        let storage = lock_storage(&state);
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

    // CO-380: billing stub — account creation triggers a billing.account_created event.
    state.core.eda_bus.publish(crate::eda::Event::new(
        "billing.account_created",
        None,
        Some(user.id.clone()),
        serde_json::json!({ "tier": user.tier }),
        crate::eda::Visibility::UserOnly,
    ));

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
    if !state.core.config.allows_uat_login() {
        return Err(AppError::NotFound("Not found".into()));
    }
    password_login_handler(State(state), Json(req)).await
}

/// CO-177: tells the UI whether Google OAuth is configured on this deploy.
/// Lets the login modal hide the "Continuar com Google" button when the
/// `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` env vars aren't set —
/// avoids a button that lands on 503.
pub(super) async fn google_status_handler() -> Json<GoogleStatusResponse> {
    let configured =
        std::env::var("GOOGLE_CLIENT_ID").is_ok() && std::env::var("GOOGLE_CLIENT_SECRET").is_ok();
    Json(GoogleStatusResponse { configured })
}

/// CO-303: `GET /api/v1/auth/login-options` — available authentication methods.
///
/// The SPA calls this on modal mount to decide which sign-in tabs to render:
/// - `magic_code`: always true (email/code is the primary path)
/// - `password`: true when `allows_uat_login()` (CO_ENV=uat|test) — reveals
///   the "Admin sign-in (password)" tab in non-prod environments
/// - `google`: true when Google OAuth env vars are configured
pub(super) async fn login_options_handler(
    State(state): State<AppState>,
) -> Json<LoginOptionsResponse> {
    let google =
        std::env::var("GOOGLE_CLIENT_ID").is_ok() && std::env::var("GOOGLE_CLIENT_SECRET").is_ok();
    Json(LoginOptionsResponse {
        magic_code: true,
        password: state.core.config.allows_uat_login(),
        google,
    })
}

// --- CO-206: cross-apex SSO handover ---

#[derive(serde::Deserialize)]
pub(super) struct CoHandoverQuery {
    pub return_to: Option<String>,
}

/// CO-206: `GET /auth/co-handover?return_to=<receiver_url>`
///
/// Issues a short-lived (60s) ES256-signed handover token and redirects to
/// `<receiver_url>?co_token=<jwt>`. The receiver validates the token via CO's
/// JWKS at `/.well-known/jwks.json` — no shared secret required.
///
/// Route is behind `require_auth`, so unauthenticated requests return 401.
/// An `return_to` host not on the safelist returns 400.
pub(super) async fn co_handover_handler(
    State(state): State<AppState>,
    crate::auth::UserId(user_id): crate::auth::UserId,
    Query(params): Query<CoHandoverQuery>,
) -> Result<Response, AppError> {
    let return_to = params.return_to.as_deref().unwrap_or("");
    if return_to.is_empty() || !crate::recovery_routes::is_allowed_return_to(return_to) {
        return Err(AppError::BadRequest(
            "return_to is missing or not in the safelist".into(),
        ));
    }
    let user = {
        let storage = state.core.storage.lock();
        storage
            .get_user_by_id(&user_id)
            .ok_or_else(|| AppError::Internal("User not found".into()))?
    };
    let redirect_url = crate::auth::maybe_attach_co_handover_token(
        return_to,
        &user.id,
        &user.email,
        &user.tier,
        &state.integrations.jwt_key,
    );
    Ok((
        StatusCode::SEE_OTHER,
        [(header::LOCATION, redirect_url)],
        (),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UserStatsResponse serializes with expected fields.
    #[test]
    fn test_user_stats_response_serializes() {
        let r = UserStatsResponse {
            user_id: "usr-123".to_string(),
            universes: vec![UniverseStat {
                key: "my-uni".to_string(),
                name: "My Universe".to_string(),
                entries: 42,
                pages: 10,
                tasks: 5,
                content_count: 42,
                is_public: false,
            }],
            total_universes: 1,
            total_entries: 42,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["user_id"], "usr-123");
        assert_eq!(json["total_universes"], 1);
        assert_eq!(json["universes"][0]["key"], "my-uni");
        assert_eq!(json["universes"][0]["entries"], 42);
    }

    /// GoogleStatusResponse serializes with configured field.
    #[test]
    fn test_google_status_response_serializes() {
        let r = GoogleStatusResponse { configured: false };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["configured"], false);
    }

    /// CO-303: LoginOptionsResponse serializes correctly.
    #[test]
    fn test_login_options_response_serializes() {
        let r = LoginOptionsResponse {
            magic_code: true,
            password: false,
            google: false,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["magic_code"], true);
        assert_eq!(json["password"], false);
        assert_eq!(json["google"], false);
    }

    /// CO-303: dev_code is absent from LoginResponse when not set.
    #[test]
    fn test_login_response_dev_code_omitted_when_none() {
        let r = crate::models::LoginResponse {
            message: "sent".into(),
            dev_code: None,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["message"], "sent");
        assert!(
            json.get("dev_code").is_none(),
            "dev_code must be absent in prod"
        );
    }

    /// CO-303: dev_code is included in LoginResponse when set.
    #[test]
    fn test_login_response_dev_code_present() {
        let r = crate::models::LoginResponse {
            message: "sent".into(),
            dev_code: Some("123456".into()),
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["dev_code"], "123456");
    }
}
