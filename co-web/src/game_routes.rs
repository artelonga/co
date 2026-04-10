use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rand::Rng;

use crate::auth::{Claims, UserId};
use crate::game_models::*;
use crate::server::AppState;

// ---- Constants ----

const INITIAL_BALANCE: u64 = 10_000;
/// Verification code TTL in seconds (5 minutes)
const CODE_TTL_SECS: i64 = 300;
/// Rate limit window in seconds (15 minutes)
const RATE_WINDOW_SECS: i64 = 900;
/// Max code requests per window
const RATE_MAX_REQUESTS: u32 = 3;
/// Max wrong attempts before code is invalidated
const MAX_ATTEMPTS: u32 = 3;
/// JWT TTL in seconds (7 days)
const JWT_TTL_SECS: i64 = 7 * 24 * 3600;

// ---- Error helpers ----

fn server_error(msg: String) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error: msg }),
    )
}

fn not_found(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

fn bad_request_json(msg: String) -> (StatusCode, Json<serde_json::Value>) {
    let body = ErrorResponse { error: msg };
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::to_value(body).unwrap()),
    )
}

// ---- Validation helpers ----

fn is_valid_username(username: &str) -> bool {
    let len = username.len();
    (3..=20).contains(&len)
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn jwt_secret() -> String {
    std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-me".into())
}

// ---- Health (unauthenticated) ----

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: "0.1.0".to_string(),
    })
}

// ---- Plugins (unauthenticated) ----

pub async fn list_plugins(State(state): State<AppState>) -> Json<Vec<PluginListEntry>> {
    let entries: Vec<PluginListEntry> = state
        .plugin_registry
        .iter()
        .map(|p| {
            let m = p.manifest();
            PluginListEntry {
                name: m.name.clone(),
                version: m.version.clone(),
                description: m.description.clone(),
            }
        })
        .collect();
    Json(entries)
}

// ---- Auth: Step 1 -- request a verification code ----

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    let email = req.email.to_lowercase();
    let storage = state.game_storage.clone();
    let mail = state.mail.clone();

    // Run all blocking storage ops in a blocking task
    let result = tokio::task::spawn_blocking(move || {
        let now = chrono::Utc::now().timestamp();

        // --- Rate limiting ---
        let rate_json = storage.get_rate_limit(&email)?;
        let mut rate: RateLimitData = rate_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(RateLimitData {
                count: 0,
                window_start: now,
            });

        if now - rate.window_start >= RATE_WINDOW_SECS {
            // Window expired -- reset
            rate = RateLimitData {
                count: 0,
                window_start: now,
            };
        }

        if rate.count >= RATE_MAX_REQUESTS {
            // Return Ok(None) -- we still respond 200 to avoid enumeration
            return Ok::<_, game_core::GameError>(None);
        }

        rate.count += 1;
        storage.save_rate_limit(&email, &serde_json::to_string(&rate).unwrap_or_default())?;

        // --- Look up user by email ---
        let user_id = storage.get_user_id_by_email(&email)?;
        if user_id.is_none() {
            // Email not registered -- silently succeed (no enumeration)
            return Ok(None);
        }
        let user_id = user_id.unwrap();

        // --- Generate 6-digit code ---
        let code: String = {
            let n: u32 = rand::thread_rng().gen_range(0..1_000_000);
            format!("{n:06}")
        };

        let verify = VerifyCodeData {
            code: code.clone(),
            user_id,
            expires_at: now + CODE_TTL_SECS,
            attempts: 0,
        };
        storage.save_verify_code(&email, &serde_json::to_string(&verify).unwrap_or_default())?;

        Ok(Some((email.clone(), code)))
    })
    .await;

    match result {
        Ok(Ok(Some((email, code)))) => {
            let body = format!("Your verification code is: {code}\nIt expires in 5 minutes.");
            let _ = mail.send(&email, "Your login code", &body);
        }
        Ok(Ok(None)) => {
            // Rate limited or unknown email -- still 200, no leak
        }
        Ok(Err(e)) => {
            eprintln!("login storage error: {e}");
        }
        Err(e) => {
            eprintln!("login task error: {e}");
        }
    }

    // Always 200 -- no email enumeration
    StatusCode::OK
}

// ---- Auth: Step 2 -- verify code and issue JWT ----

pub async fn verify(
    State(state): State<AppState>,
    Json(req): Json<VerifyRequest>,
) -> impl IntoResponse {
    let email = req.email.to_lowercase();
    let code = req.code.clone();
    let storage = state.game_storage.clone();

    let blocking_result = tokio::task::spawn_blocking(move || {
        let now = chrono::Utc::now().timestamp();

        let json = match storage.get_verify_code(&email)? {
            Some(j) => j,
            None => {
                return Ok::<_, game_core::GameError>(Err((
                    "gone",
                    0u32,
                    String::new(),
                    String::new(),
                )));
            }
        };

        let mut entry: VerifyCodeData = serde_json::from_str(&json)
            .map_err(|e| game_core::GameError::Storage(format!("Decode error: {e}")))?;

        // Expired?
        if now > entry.expires_at {
            let _ = storage.delete_verify_code(&email);
            return Ok(Err(("expired", 0, String::new(), String::new())));
        }

        // Wrong code?
        if entry.code != code {
            entry.attempts += 1;
            if entry.attempts >= MAX_ATTEMPTS {
                let _ = storage.delete_verify_code(&email);
                return Ok(Err(("exhausted", 0, String::new(), String::new())));
            }
            storage.save_verify_code(&email, &serde_json::to_string(&entry).unwrap_or_default())?;
            let remaining = MAX_ATTEMPTS - entry.attempts;
            return Ok(Err(("wrong", remaining, String::new(), String::new())));
        }

        // Correct -- delete code and fetch user
        let _ = storage.delete_verify_code(&email);

        let user = storage
            .get_user_by_id(&entry.user_id)?
            .ok_or_else(|| game_core::GameError::Storage("User not found".into()))?;

        Ok(Ok((user, email)))
    })
    .await;

    let (user, email) = match blocking_result {
        Ok(Ok(Ok(v))) => v,
        Ok(Ok(Err((reason, remaining, _, _)))) => {
            return match reason {
                "gone" | "expired" => (
                    StatusCode::GONE,
                    [(header::CONTENT_TYPE, "application/json")],
                    axum::body::Body::from(
                        serde_json::to_string(&ErrorResponse {
                            error: "Code expired, request a new one".into(),
                        })
                        .unwrap_or_default(),
                    ),
                )
                    .into_response(),
                "exhausted" => (
                    StatusCode::UNAUTHORIZED,
                    [(header::CONTENT_TYPE, "application/json")],
                    axum::body::Body::from(
                        serde_json::to_string(&ErrorResponse {
                            error: "Code expired, request a new one".into(),
                        })
                        .unwrap_or_default(),
                    ),
                )
                    .into_response(),
                _ => {
                    // "wrong"
                    (
                        StatusCode::UNAUTHORIZED,
                        [(header::CONTENT_TYPE, "application/json")],
                        axum::body::Body::from(
                            serde_json::to_string(&WrongCodeResponse {
                                error: "Invalid code".into(),
                                remaining_attempts: remaining,
                            })
                            .unwrap_or_default(),
                        ),
                    )
                        .into_response()
                }
            };
        }
        Ok(Err(e)) => {
            eprintln!("verify storage error: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
                .into_response();
        }
        Err(e) => {
            eprintln!("verify task error: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
                .into_response();
        }
    };

    // Build JWT
    let secret = jwt_secret();
    let now_ts = chrono::Utc::now().timestamp() as usize;
    let exp = now_ts + JWT_TTL_SECS as usize;

    let claims = Claims {
        sub: user.user_id.clone(),
        email: email.clone(),
        tier: "player".into(),
        usuario: String::new(),
        papel: String::new(),
        exp,
        iat: now_ts,
    };

    let token = match encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    ) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("JWT encode error: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
                .into_response();
        }
    };

    let cookie = format!(
        "session={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={JWT_TTL_SECS}"
    );

    let body = VerifyResponse {
        user_id: user.user_id,
        email,
        display_name: user.display_name,
        expires_at: exp as i64,
    };

    (
        StatusCode::OK,
        [
            (header::SET_COOKIE, cookie),
            (header::CONTENT_TYPE, "application/json".into()),
        ],
        Json(body),
    )
        .into_response()
}

// ---- Auth: registration ----

pub async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), (StatusCode, Json<serde_json::Value>)> {
    // Validate input
    let mut field_errors = Vec::new();

    if !is_valid_username(&req.username) {
        field_errors.push(FieldError {
            field: "username".to_string(),
            message: "Must be 3-20 characters, alphanumeric and underscore only".to_string(),
        });
    }

    if req.password.len() < 8 {
        field_errors.push(FieldError {
            field: "password".to_string(),
            message: "Must be at least 8 characters".to_string(),
        });
    }

    let email_str = req.email.as_deref().unwrap_or("");
    if email_str.is_empty() || !email_str.contains('@') {
        field_errors.push(FieldError {
            field: "email".to_string(),
            message: "Must be a valid email address".to_string(),
        });
    }

    if !field_errors.is_empty() {
        let body = ValidationErrorResponse {
            error: "Bad Request".to_string(),
            fields: field_errors,
        };
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::to_value(body).unwrap()),
        ));
    }

    // Check username uniqueness
    let storage = state.game_storage.clone();
    let username = req.username.clone();
    let existing = tokio::task::spawn_blocking({
        let storage = storage.clone();
        let username = username.clone();
        move || storage.get_user_id_by_username(&username)
    })
    .await
    .map_err(|e| bad_request_json(format!("Task error: {}", e)))?
    .map_err(|e| bad_request_json(format!("Storage error: {}", e)))?;

    if existing.is_some() {
        let body = ErrorResponse {
            error: "Username already taken".to_string(),
        };
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::to_value(body).unwrap()),
        ));
    }

    // Hash password with Argon2id
    let password = req.password.clone();
    let password_hash = tokio::task::spawn_blocking(move || {
        use argon2::Argon2;
        use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| format!("Password hashing failed: {}", e))
    })
    .await
    .map_err(|e| bad_request_json(format!("Task error: {}", e)))?
    .map_err(bad_request_json)?;

    // Create user profile and wallet
    let display_name = req.display_name.clone();
    let email = req.email.as_deref().unwrap_or("").to_lowercase();
    let result = tokio::task::spawn_blocking(move || {
        let now = chrono::Utc::now().timestamp();
        let user_id = nanoid::nanoid!(12);

        let user = game_core::storage::schema::UserProfile {
            user_id: user_id.clone(),
            username: username.clone(),
            display_name: display_name.clone(),
            password_hash,
            email: email.clone(),
            created_at: now,
            last_login_at: now,
            stats: Some(game_core::storage::schema::UserStats::default()),
        };

        storage.save_user_by_id(&user)?;
        storage.save_username_index(&username, &user_id)?;
        storage.save_email_index(&email, &user_id)?;

        // Initialize wallet with 10,000 chips
        let wallet = game_core::storage::schema::Wallet {
            user_id: user_id.clone(),
            balance: INITIAL_BALANCE,
            last_updated: now,
        };
        storage.save_wallet_for_user(&user_id, &wallet)?;

        // Record initial grant transaction
        let tx = game_core::storage::schema::Transaction {
            tx_id: nanoid::nanoid!(10),
            tx_type: game_core::storage::schema::TransactionType::TxInitialGrant as i32,
            amount: INITIAL_BALANCE as i64,
            balance_after: INITIAL_BALANCE,
            timestamp: now,
            hand_id: String::new(),
            description: "Welcome bonus".to_string(),
        };
        storage.save_transaction(&tx)?;

        Ok::<_, game_core::GameError>((user_id, username, display_name))
    })
    .await
    .map_err(|e| bad_request_json(format!("Task error: {}", e)))?
    .map_err(|e| bad_request_json(format!("Storage error: {}", e)))?;

    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            user_id: result.0,
            username: result.1,
            display_name: result.2,
        }),
    ))
}

// ---- Legacy login (Godot client) ----

pub async fn legacy_login(
    State(state): State<AppState>,
    Json(req): Json<LegacyLoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorResponse>)> {
    let storage = state.game_storage.clone();
    let name = req.name.clone();

    let user = tokio::task::spawn_blocking(move || {
        let user = match storage.get_user() {
            Ok(Some(u)) => u,
            Ok(None) => {
                return Err("No user found. Run the game CLI first to initialize.".to_string());
            }
            Err(e) => return Err(format!("Storage error: {}", e)),
        };

        if !name.is_empty() && name != user.display_name {
            let mut updated = user.clone();
            updated.display_name = name;
            updated.last_login_at = chrono::Utc::now().timestamp();
            if let Err(e) = storage.save_user(&updated) {
                return Err(format!("Failed to update user: {}", e));
            }
            return Ok(updated);
        }

        Ok(user)
    })
    .await
    .map_err(|e| server_error(format!("Task join error: {}", e)))?
    .map_err(server_error)?;

    // Issue JWT
    let secret = jwt_secret();
    let now_ts = chrono::Utc::now().timestamp() as usize;
    let exp = now_ts + JWT_TTL_SECS as usize;

    let claims = Claims {
        sub: user.user_id.clone(),
        email: user.email.clone(),
        tier: "player".into(),
        usuario: String::new(),
        papel: String::new(),
        exp,
        iat: now_ts,
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| server_error(format!("JWT encode error: {e}")))?;

    Ok(Json(LoginResponse {
        token,
        user_id: user.user_id,
        display_name: user.display_name,
    }))
}

// ---- Profile (authenticated) ----

pub async fn get_profile(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
) -> Result<Json<ProfileResponse>, (StatusCode, Json<ErrorResponse>)> {
    let storage = state.game_storage.clone();
    let uid = user_id.0;

    let user = tokio::task::spawn_blocking(move || storage.get_user_by_id(&uid))
        .await
        .map_err(|e| server_error(format!("Task error: {}", e)))?
        .map_err(|e| server_error(format!("Storage error: {}", e)))?
        .ok_or_else(|| not_found("User not found"))?;

    Ok(Json(ProfileResponse {
        user_id: user.user_id,
        display_name: user.display_name,
        created_at: user.created_at,
        last_login_at: user.last_login_at,
        stats: user.stats.map(|s| StatsResponse {
            hands_played: s.hands_played,
            hands_won: s.hands_won,
            total_chips_won: s.total_chips_won,
            total_chips_lost: s.total_chips_lost,
            biggest_pot_won: s.biggest_pot_won,
            best_hand_rank: s.best_hand_rank,
            sessions_played: s.sessions_played,
        }),
    }))
}

// ---- Wallet (authenticated) ----

pub async fn get_wallet(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
) -> Result<Json<WalletResponse>, (StatusCode, Json<ErrorResponse>)> {
    let storage = state.game_storage.clone();
    let uid = user_id.0;

    let wallet = tokio::task::spawn_blocking(move || storage.get_wallet_for_user(&uid))
        .await
        .map_err(|e| server_error(format!("Task error: {}", e)))?
        .map_err(|e| server_error(format!("Storage error: {}", e)))?;

    let balance = wallet.map(|w| w.balance).unwrap_or(0);
    Ok(Json(WalletResponse { balance }))
}

// ---- Game Results (authenticated) ----

pub async fn record_game_result(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(game_name): Path<String>,
    Json(req): Json<GameResultRequest>,
) -> Result<Json<GameStatsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let storage = state.game_storage.clone();
    let gname = game_name.clone();
    let uid = user_id.0;
    let score = req.score;

    let stats = tokio::task::spawn_blocking(move || {
        let mut stats = storage
            .get_game_stats(&gname, &uid)
            .ok()
            .flatten()
            .unwrap_or_else(|| game_core::storage::schema::GameStats {
                game_name: gname.clone(),
                high_score: 0,
                games_played: 0,
                last_played_at: 0,
            });

        stats.games_played += 1;
        if score > stats.high_score {
            stats.high_score = score;
        }
        stats.last_played_at = chrono::Utc::now().timestamp();

        storage
            .save_game_stats(&uid, &stats)
            .map_err(|e| format!("Failed to save game stats: {}", e))?;

        Ok::<_, String>(stats)
    })
    .await
    .map_err(|e| server_error(format!("Task error: {}", e)))?
    .map_err(server_error)?;

    Ok(Json(GameStatsResponse {
        game_name: stats.game_name,
        high_score: stats.high_score,
        games_played: stats.games_played,
        last_played_at: stats.last_played_at,
    }))
}

pub async fn get_game_stats(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(game_name): Path<String>,
) -> Result<Json<GameStatsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let storage = state.game_storage.clone();
    let uid = user_id.0;

    let stats = tokio::task::spawn_blocking(move || storage.get_game_stats(&game_name, &uid))
        .await
        .map_err(|e| server_error(format!("Task error: {}", e)))?
        .map_err(|e| server_error(format!("Storage error: {}", e)))?
        .ok_or_else(|| not_found("No stats for this game"))?;

    Ok(Json(GameStatsResponse {
        game_name: stats.game_name,
        high_score: stats.high_score,
        games_played: stats.games_played,
        last_played_at: stats.last_played_at,
    }))
}

// ---- Leaderboard (unauthenticated) ----

pub async fn get_leaderboard(
    State(state): State<AppState>,
    Path(game_name): Path<String>,
    Query(params): Query<LeaderboardQuery>,
) -> Result<Json<Vec<LeaderboardEntry>>, (StatusCode, Json<ErrorResponse>)> {
    let limit = params.limit.unwrap_or(20).min(100) as usize;
    let storage = state.game_storage.clone();
    let gname = game_name.clone();

    let entries = tokio::task::spawn_blocking(move || {
        let stats_with_ids = storage.list_game_stats_with_user_ids(&gname)?;

        let mut scored: Vec<(String, i64, u64)> = stats_with_ids
            .into_iter()
            .map(|(user_id, stats)| (user_id, stats.high_score, stats.games_played))
            .collect();

        scored.sort_by_key(|e| std::cmp::Reverse(e.1));
        scored.truncate(limit);

        let mut results = Vec::with_capacity(scored.len());
        for (rank, (user_id, high_score, games_played)) in scored.into_iter().enumerate() {
            let user = storage.get_user_by_id(&user_id).ok().flatten();
            let display_name = user
                .as_ref()
                .map(|u| u.display_name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let username = user
                .as_ref()
                .map(|u| u.username.clone())
                .unwrap_or_default();

            results.push(LeaderboardEntry {
                rank: (rank + 1) as u32,
                user_id,
                display_name,
                username,
                high_score,
                games_played,
            });
        }

        Ok::<_, game_core::GameError>(results)
    })
    .await
    .map_err(|e| server_error(format!("Task error: {}", e)))?
    .map_err(|e| server_error(format!("Storage error: {}", e)))?;

    Ok(Json(entries))
}

// ---- Global Leaderboard (unauthenticated) ----

pub async fn get_global_leaderboard(
    State(state): State<AppState>,
    Query(params): Query<LeaderboardQuery>,
) -> Result<Json<Vec<GlobalLeaderboardEntry>>, (StatusCode, Json<ErrorResponse>)> {
    let limit = params.limit.unwrap_or(10).min(100) as usize;
    let storage = state.game_storage.clone();

    let entries = tokio::task::spawn_blocking(move || {
        // Stat keys have the format "{game_name}:{user_id}".
        // Collect unique user IDs from the keys, then aggregate their scores.
        let keys = storage
            .list_game_stat_keys()
            .map_err(|e| format!("Storage error: {e}"))?;

        let mut user_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for key in &keys {
            if let Some(pos) = key.find(':') {
                user_ids.insert(key[pos + 1..].to_string());
            }
        }

        let mut aggregated: Vec<(String, String, String, i64, u64)> = Vec::new();

        for uid in user_ids {
            let stats_list = storage.list_game_stats_by_user(&uid).unwrap_or_default();
            if stats_list.is_empty() {
                continue;
            }
            let total_score: i64 = stats_list.iter().map(|s| s.high_score).sum();
            let total_played: u64 = stats_list.iter().map(|s| s.games_played).sum();
            if total_score == 0 && total_played == 0 {
                continue;
            }

            let user = storage.get_user_by_id(&uid).ok().flatten();
            let display_name = user
                .as_ref()
                .map(|u| u.display_name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let username = user
                .as_ref()
                .map(|u| u.username.clone())
                .unwrap_or_default();

            aggregated.push((uid, display_name, username, total_score, total_played));
        }

        aggregated.sort_by_key(|e| std::cmp::Reverse(e.3));
        aggregated.truncate(limit);

        let results = aggregated
            .into_iter()
            .enumerate()
            .map(
                |(i, (user_id, display_name, username, total_score, games_played))| {
                    GlobalLeaderboardEntry {
                        rank: (i + 1) as u32,
                        user_id,
                        display_name,
                        username,
                        total_score,
                        games_played,
                    }
                },
            )
            .collect();

        Ok::<_, String>(results)
    })
    .await
    .map_err(|e| server_error(format!("Task error: {e}")))?
    .map_err(server_error)?;

    Ok(Json(entries))
}

// ---- Recent Activity Feed (unauthenticated) ----

pub async fn get_recent_activity(
    State(state): State<AppState>,
    Query(params): Query<LeaderboardQuery>,
) -> Result<Json<Vec<RecentActivityEntry>>, (StatusCode, Json<ErrorResponse>)> {
    let limit = params.limit.unwrap_or(20).min(100) as usize;
    let storage = state.game_storage.clone();

    let entries = tokio::task::spawn_blocking(move || {
        let keys = storage
            .list_game_stat_keys()
            .map_err(|e| format!("Storage error: {e}"))?;

        let mut user_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for key in &keys {
            if let Some(pos) = key.find(':') {
                user_ids.insert(key[pos + 1..].to_string());
            }
        }

        let mut activity: Vec<RecentActivityEntry> = Vec::new();

        for uid in user_ids {
            let stats_list = storage.list_game_stats_by_user(&uid).unwrap_or_default();

            let user = storage.get_user_by_id(&uid).ok().flatten();
            let display_name = user
                .as_ref()
                .map(|u| u.display_name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let username = user
                .as_ref()
                .map(|u| u.username.clone())
                .unwrap_or_default();

            for stat in stats_list {
                if stat.games_played == 0 {
                    continue;
                }
                activity.push(RecentActivityEntry {
                    user_id: uid.clone(),
                    display_name: display_name.clone(),
                    username: username.clone(),
                    game_name: stat.game_name.clone(),
                    score: stat.high_score,
                    played_at: stat.last_played_at,
                });
            }
        }

        activity.sort_by_key(|e| std::cmp::Reverse(e.played_at));
        activity.truncate(limit);

        Ok::<_, String>(activity)
    })
    .await
    .map_err(|e| server_error(format!("Task error: {e}")))?
    .map_err(server_error)?;

    Ok(Json(entries))
}

// ---- Player Profile (unauthenticated) ----

pub async fn get_player_profile(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<PlayerProfileResponse>, (StatusCode, Json<ErrorResponse>)> {
    let storage = state.game_storage.clone();
    let uname = username.clone();

    let result = tokio::task::spawn_blocking(move || {
        let user_id = storage
            .get_user_id_by_username(&uname)?
            .ok_or_else(|| game_core::GameError::Storage("User not found".into()))?;

        let user = storage
            .get_user_by_id(&user_id)?
            .ok_or_else(|| game_core::GameError::Storage("User not found".into()))?;

        let stats = storage.list_game_stats_by_user(&user_id)?;

        Ok::<_, game_core::GameError>((user, stats))
    })
    .await
    .map_err(|e| server_error(format!("Task error: {}", e)))?;

    match result {
        Ok((user, stats)) => {
            let game_stats = stats
                .into_iter()
                .map(|s| GameStatsResponse {
                    game_name: s.game_name,
                    high_score: s.high_score,
                    games_played: s.games_played,
                    last_played_at: s.last_played_at,
                })
                .collect();

            Ok(Json(PlayerProfileResponse {
                user_id: user.user_id,
                username: user.username,
                display_name: user.display_name,
                created_at: user.created_at,
                game_stats,
                universes: vec![],
            }))
        }
        Err(_) => Err(not_found("Player not found")),
    }
}
