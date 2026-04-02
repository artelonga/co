use axum::extract::{Json, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Router, middleware};

use crate::auth::{self, UserId};
use crate::error::AppError;
use crate::quilombo_models::*;
use crate::quilombo_permissoes::tem_permissao;
use crate::quilombo_storage;
use crate::server::AppState;
use crate::storage::Storage;

fn lock_storage(state: &AppState) -> Result<std::sync::MutexGuard<'_, Storage>, AppError> {
    state
        .storage
        .lock()
        .map_err(|_| AppError::Internal("Storage lock failed".into()))
}

fn relatos_dir() -> String {
    std::env::var("QUILOMBO_RELATOS_DIR").unwrap_or_else(|_| "relatos".to_string())
}

fn paginas_dir() -> String {
    std::env::var("QUILOMBO_PAGINAS_DIR").unwrap_or_else(|_| "jardim".to_string())
}

/// Look up a quilombo user from their co-web user ID.
fn lookup_quilombo_user(storage: &Storage, user_id: &str) -> Result<Usuario, AppError> {
    quilombo_storage::obter_usuario_por_id(storage.conn(), user_id)
        .ok_or_else(|| AppError::NotFound("Quilombo user not found".into()))
}

// --- Router ---

pub fn router() -> Router<AppState> {
    let public = Router::new()
        // Auth
        .route("/auth/login", post(login_handler))
        .route("/auth/cadastro", post(cadastro_handler))
        // Content (public read)
        .route("/publicacoes", get(listar_publicacoes))
        .route("/publicacoes/{slug}", get(obter_publicacao))
        .route("/paginas/{slug}", get(obter_pagina))
        // Events (public read)
        .route("/eventos", get(listar_eventos_handler))
        .route("/eventos/{id}", get(obter_evento_handler))
        // Missions (public read)
        .route("/missoes", get(listar_missoes_handler))
        .route("/missoes/{id}", get(obter_missao_handler))
        // Members (public)
        .route("/membros", get(listar_membros_handler))
        .route("/membros/{usuario}", get(obter_membro_handler))
        // Comments (public read + anonymous create)
        .route(
            "/comentarios",
            get(listar_comentarios_handler).post(criar_comentario_handler),
        )
        // Contact form (public)
        .route("/contato", post(contato_handler))
        // Tags
        .route("/tags", get(listar_tags_handler))
        .route("/tags/{tag}", get(publicacoes_por_tag_handler));

    let authenticated = Router::new()
        // Profile
        .route(
            "/perfil",
            get(obter_perfil_handler).put(atualizar_perfil_handler),
        )
        // Missions (write)
        .route("/missoes/criar", post(criar_missao_handler))
        .route("/missoes/{id}/participar", post(participar_missao_handler))
        .route(
            "/missoes/{id}/participacoes/{uid}",
            put(atualizar_participacao_handler),
        )
        // Events (admin write)
        .route("/eventos/criar", post(criar_evento_handler))
        .route("/eventos/{id}/editar", put(atualizar_evento_handler))
        .route("/eventos/{id}/excluir", post(excluir_evento_handler))
        // Messages
        .route(
            "/mensagens",
            get(listar_mensagens_handler).post(criar_mensagem_handler),
        )
        // Admin
        .route("/admin/atividades", get(listar_atividades_handler))
        .layer(middleware::from_fn(auth::require_auth));

    Router::new().merge(public).merge(authenticated)
}

// --- Auth Handlers ---

async fn login_handler(
    State(state): State<AppState>,
    Json(body): Json<LoginUsuario>,
) -> Result<Response, AppError> {
    let usuario = body.usuario.trim().to_lowercase();
    if usuario.is_empty() {
        return Err(AppError::BadRequest("Username is required".into()));
    }

    // Rate limit check
    {
        let auth = state
            .auth_store
            .lock()
            .map_err(|_| AppError::Internal("Auth lock failed".into()))?;
        if !auth.check_rate_limit(&usuario)? {
            return Err(AppError::TooManyRequests(
                "Too many login attempts. Please wait.".into(),
            ));
        }
        auth.record_request(&usuario)?;
    }

    let storage = lock_storage(&state)?;
    let (user, senha_hash) = quilombo_storage::obter_usuario_por_nome(storage.conn(), &usuario)
        .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?;

    // Verify password with argon2 0.5 (RustCrypto)
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHash, PasswordVerifier};

    let parsed_hash = PasswordHash::new(&senha_hash)
        .map_err(|_| AppError::Internal("Invalid password hash in database".into()))?;
    Argon2::default()
        .verify_password(body.senha.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized("Invalid credentials".into()))?;

    // Sign JWT
    let jwt_secret = auth::jwt_secret();
    let (token, _expires_at) = auth::sign_jwt_quilombo(
        &user.id,
        &user.usuario,
        &user.papel.to_string(),
        &jwt_secret,
    )?;

    let cookie =
        format!("session={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=604800");

    let response_body = LoginResponse {
        token: token.clone(),
        usuario: UsuarioResumo {
            id: user.id.clone(),
            usuario: user.usuario,
            nome: user.nome,
            papel: user.papel,
        },
    };

    quilombo_storage::registrar_atividade(
        storage.conn(),
        "login",
        "usuario",
        Some(&user.id),
        Some(&user.id),
        None,
    );

    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response_body),
    )
        .into_response())
}

async fn cadastro_handler(
    State(state): State<AppState>,
    Json(body): Json<CriarUsuario>,
) -> Result<Response, AppError> {
    let usuario = body.usuario.trim().to_lowercase();

    // Validate username
    if usuario.len() < 3 || usuario.len() > 30 {
        return Err(AppError::BadRequest(
            "Username must be 3-30 characters".into(),
        ));
    }
    if !usuario
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(AppError::BadRequest(
            "Username must contain only lowercase letters, numbers, and underscores".into(),
        ));
    }

    // Validate name
    let nome = body.nome.trim().to_string();
    if nome.len() < 2 {
        return Err(AppError::BadRequest(
            "Name must be at least 2 characters".into(),
        ));
    }

    // Validate password
    if body.senha.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".into(),
        ));
    }

    // Hash password with argon2 0.5 (RustCrypto)
    use argon2::Argon2;
    use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let senha_hash = argon2
        .hash_password(body.senha.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(format!("Password hash failed: {e}")))?;

    let id = uuid::Uuid::new_v4().to_string();

    let storage = lock_storage(&state)?;

    // Check uniqueness
    if quilombo_storage::obter_usuario_por_nome(storage.conn(), &usuario).is_some() {
        return Err(AppError::Conflict("Username already taken".into()));
    }

    let user = quilombo_storage::criar_usuario(storage.conn(), &id, &usuario, &nome, &senha_hash)
        .map_err(AppError::Internal)?;

    // Sign JWT
    let jwt_secret = auth::jwt_secret();
    let (token, _) = auth::sign_jwt_quilombo(
        &user.id,
        &user.usuario,
        &user.papel.to_string(),
        &jwt_secret,
    )?;

    let cookie =
        format!("session={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=604800");

    quilombo_storage::registrar_atividade(
        storage.conn(),
        "criar",
        "usuario",
        Some(&user.id),
        Some(&user.id),
        None,
    );

    let response_body = LoginResponse {
        token: token.clone(),
        usuario: UsuarioResumo {
            id: user.id,
            usuario: user.usuario,
            nome: user.nome,
            papel: user.papel,
        },
    };

    Ok((
        StatusCode::CREATED,
        [(header::SET_COOKIE, cookie)],
        Json(response_body),
    )
        .into_response())
}

// --- Content Handlers (filesystem markdown) ---

async fn listar_publicacoes() -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let quilombo_dir = std::env::var("QUILOMBO_DIR").unwrap_or_else(|_| "quilombo".to_string());
    let posts_dir = std::path::Path::new(&quilombo_dir).join(relatos_dir());

    let mut posts = Vec::new();
    let entries = std::fs::read_dir(&posts_dir)
        .into_iter()
        .flatten()
        .flatten();
    for entry in entries {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(fm) = parse_frontmatter(&content) else {
            continue;
        };
        if fm.get("publicado").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        let mut post = fm;
        if let (Some(body_start), Some(obj)) = (find_body_start(&content), post.as_object_mut()) {
            obj.insert(
                "corpo".to_string(),
                serde_json::Value::String(content[body_start..].to_string()),
            );
        }
        posts.push(post);
    }

    // Sort by date descending
    posts.sort_by(|a, b| {
        let da = a.get("data").and_then(|v| v.as_str()).unwrap_or("");
        let db = b.get("data").and_then(|v| v.as_str()).unwrap_or("");
        db.cmp(da)
    });

    Ok(Json(posts))
}

async fn obter_publicacao(Path(slug): Path<String>) -> Result<Json<serde_json::Value>, AppError> {
    let quilombo_dir = std::env::var("QUILOMBO_DIR").unwrap_or_else(|_| "quilombo".to_string());
    let path = std::path::Path::new(&quilombo_dir)
        .join(relatos_dir())
        .join(format!("{slug}.md"));

    let content = std::fs::read_to_string(&path)
        .map_err(|_| AppError::NotFound(format!("Post '{slug}' not found")))?;

    let mut fm = parse_frontmatter(&content)
        .ok_or_else(|| AppError::Internal("Invalid frontmatter".into()))?;

    if let (Some(body_start), Some(obj)) = (find_body_start(&content), fm.as_object_mut()) {
        obj.insert(
            "corpo".to_string(),
            serde_json::Value::String(content[body_start..].to_string()),
        );
    }

    Ok(Json(fm))
}

async fn obter_pagina(Path(slug): Path<String>) -> Result<Json<serde_json::Value>, AppError> {
    let quilombo_dir = std::env::var("QUILOMBO_DIR").unwrap_or_else(|_| "quilombo".to_string());
    let path = std::path::Path::new(&quilombo_dir)
        .join(paginas_dir())
        .join(format!("{slug}.md"));

    let content = std::fs::read_to_string(&path)
        .map_err(|_| AppError::NotFound(format!("Page '{slug}' not found")))?;

    let mut fm = parse_frontmatter(&content)
        .ok_or_else(|| AppError::Internal("Invalid frontmatter".into()))?;

    if let (Some(body_start), Some(obj)) = (find_body_start(&content), fm.as_object_mut()) {
        obj.insert(
            "corpo".to_string(),
            serde_json::Value::String(content[body_start..].to_string()),
        );
    }

    Ok(Json(fm))
}

fn parse_frontmatter(content: &str) -> Option<serde_json::Value> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let after_first = content[3..].trim_start_matches('\n');
    let end = after_first.find("---")?;
    let yaml = &after_first[..end];
    serde_yaml::from_str(yaml).ok()
}

fn find_body_start(content: &str) -> Option<usize> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let after_first = &content[3..];
    let trimmed = after_first.trim_start_matches('\n');
    let offset = 3 + (after_first.len() - trimmed.len());
    let end = trimmed.find("---")?;
    let after_closing = offset + end + 3;
    // Skip the newline after closing ---
    let body_start = content[after_closing..]
        .find('\n')
        .map(|i| after_closing + i + 1)?;
    if body_start < content.len() {
        Some(body_start)
    } else {
        None
    }
}

// --- Tags ---

async fn listar_tags_handler() -> Result<Json<Vec<String>>, AppError> {
    let quilombo_dir = std::env::var("QUILOMBO_DIR").unwrap_or_else(|_| "quilombo".to_string());
    let posts_dir = std::path::Path::new(&quilombo_dir).join(relatos_dir());

    let mut tags = std::collections::BTreeSet::new();
    let entries = std::fs::read_dir(&posts_dir)
        .into_iter()
        .flatten()
        .flatten();
    for entry in entries {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(fm) = parse_frontmatter(&content) else {
            continue;
        };
        if let Some(t) = fm.get("tags").and_then(|v| v.as_array()) {
            for tag in t.iter().filter_map(|v| v.as_str()) {
                tags.insert(tag.to_string());
            }
        }
    }

    Ok(Json(tags.into_iter().collect()))
}

async fn publicacoes_por_tag_handler(
    Path(tag): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let quilombo_dir = std::env::var("QUILOMBO_DIR").unwrap_or_else(|_| "quilombo".to_string());
    let posts_dir = std::path::Path::new(&quilombo_dir).join(relatos_dir());

    let mut posts = Vec::new();
    let entries = std::fs::read_dir(&posts_dir)
        .into_iter()
        .flatten()
        .flatten();
    for entry in entries {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(fm) = parse_frontmatter(&content) else {
            continue;
        };
        let has_tag = fm
            .get("tags")
            .and_then(|v| v.as_array())
            .is_some_and(|arr| arr.iter().any(|t| t.as_str() == Some(&tag)));
        if has_tag {
            posts.push(fm);
        }
    }

    Ok(Json(posts))
}

// --- Event Handlers ---

async fn listar_eventos_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<Evento>>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(quilombo_storage::listar_eventos(storage.conn())))
}

async fn obter_evento_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Evento>, AppError> {
    let storage = lock_storage(&state)?;
    quilombo_storage::obter_evento(storage.conn(), id)
        .map(Json)
        .ok_or_else(|| AppError::NotFound("Event not found".into()))
}

async fn criar_evento_handler(
    State(state): State<AppState>,
    user_id: UserId,
    Json(body): Json<CriarEvento>,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state)?;
    let user = lookup_quilombo_user(&storage, &user_id.0)?;

    if !tem_permissao(&user.papel, "evento:criar") {
        return Err(AppError::Forbidden("Insufficient permissions".into()));
    }

    let evento = quilombo_storage::criar_evento(storage.conn(), &body, &user.id)
        .map_err(AppError::Internal)?;

    quilombo_storage::registrar_atividade(
        storage.conn(),
        "criar",
        "evento",
        Some(&evento.id.to_string()),
        Some(&user.id),
        None,
    );

    Ok((StatusCode::CREATED, Json(evento)))
}

async fn atualizar_evento_handler(
    State(state): State<AppState>,
    user_id: UserId,
    Path(id): Path<i64>,
    Json(body): Json<AtualizarEvento>,
) -> Result<Json<Evento>, AppError> {
    let storage = lock_storage(&state)?;
    let user = lookup_quilombo_user(&storage, &user_id.0)?;

    if !tem_permissao(&user.papel, "evento:editar") {
        return Err(AppError::Forbidden("Insufficient permissions".into()));
    }

    let evento = quilombo_storage::atualizar_evento(storage.conn(), id, &body)
        .map_err(AppError::NotFound)?;

    Ok(Json(evento))
}

async fn excluir_evento_handler(
    State(state): State<AppState>,
    user_id: UserId,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    let storage = lock_storage(&state)?;
    let user = lookup_quilombo_user(&storage, &user_id.0)?;

    if !tem_permissao(&user.papel, "evento:excluir") {
        return Err(AppError::Forbidden("Insufficient permissions".into()));
    }

    quilombo_storage::excluir_evento(storage.conn(), id).map_err(AppError::NotFound)?;

    Ok(StatusCode::NO_CONTENT)
}

// --- Mission Handlers ---

async fn listar_missoes_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<Missao>>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(quilombo_storage::listar_missoes(storage.conn())))
}

async fn obter_missao_handler(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    let storage = lock_storage(&state)?;
    let missao = quilombo_storage::obter_missao(storage.conn(), id)
        .ok_or_else(|| AppError::NotFound("Mission not found".into()))?;
    let participacoes = quilombo_storage::listar_participacoes(storage.conn(), id);

    let mut result = serde_json::to_value(&missao).unwrap();
    if let Some(obj) = result.as_object_mut() {
        obj.insert(
            "participacoes".to_string(),
            serde_json::to_value(&participacoes).unwrap(),
        );
    }

    Ok(Json(result))
}

async fn criar_missao_handler(
    State(state): State<AppState>,
    user_id: UserId,
    Json(body): Json<CriarMissao>,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state)?;
    let user = lookup_quilombo_user(&storage, &user_id.0)?;

    if !tem_permissao(&user.papel, "missao:criar") {
        return Err(AppError::Forbidden("Insufficient permissions".into()));
    }

    let missao = quilombo_storage::criar_missao(storage.conn(), &body, &user.id)
        .map_err(AppError::Internal)?;

    quilombo_storage::registrar_atividade(
        storage.conn(),
        "criar",
        "missao",
        Some(&missao.id.to_string()),
        Some(&user.id),
        None,
    );

    Ok((StatusCode::CREATED, Json(missao)))
}

async fn participar_missao_handler(
    State(state): State<AppState>,
    user_id: UserId,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state)?;

    // Verify mission exists
    quilombo_storage::obter_missao(storage.conn(), id)
        .ok_or_else(|| AppError::NotFound("Mission not found".into()))?;

    let participacao = quilombo_storage::participar_missao(storage.conn(), id, &user_id.0)
        .map_err(AppError::Conflict)?;

    Ok((StatusCode::CREATED, Json(participacao)))
}

async fn atualizar_participacao_handler(
    State(state): State<AppState>,
    user_id: UserId,
    Path((id, uid)): Path<(i64, String)>,
    Json(body): Json<AtualizarParticipacao>,
) -> Result<StatusCode, AppError> {
    let storage = lock_storage(&state)?;
    let user = lookup_quilombo_user(&storage, &user_id.0)?;

    // Check: must be mission creator or admin
    let missao = quilombo_storage::obter_missao(storage.conn(), id)
        .ok_or_else(|| AppError::NotFound("Mission not found".into()))?;

    let is_creator = missao.criado_por.as_deref() == Some(&user.id);
    if !is_creator && user.papel != Papel::Admin {
        return Err(AppError::Forbidden(
            "Only mission creator or admin can approve/reject".into(),
        ));
    }

    quilombo_storage::atualizar_participacao(storage.conn(), id, &uid, &body.status)
        .map_err(AppError::NotFound)?;

    Ok(StatusCode::OK)
}

// --- Member Handlers ---

async fn listar_membros_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<Membro>>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(quilombo_storage::listar_membros(storage.conn())))
}

async fn obter_membro_handler(
    State(state): State<AppState>,
    Path(usuario): Path<String>,
) -> Result<Json<Membro>, AppError> {
    let storage = lock_storage(&state)?;
    let (user, _) = quilombo_storage::obter_usuario_por_nome(storage.conn(), &usuario)
        .ok_or_else(|| AppError::NotFound("Member not found".into()))?;

    Ok(Json(Membro {
        id: user.id,
        usuario: user.usuario,
        nome: user.nome,
        bio: user.bio,
        foto_url: user.foto_url,
        criado_em: user.criado_em,
    }))
}

// --- Comment Handlers ---

async fn listar_comentarios_handler(
    State(state): State<AppState>,
    Query(query): Query<ComentarioQuery>,
) -> Result<Json<Vec<Comentario>>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(quilombo_storage::listar_comentarios(
        storage.conn(),
        &query,
    )))
}

async fn criar_comentario_handler(
    State(state): State<AppState>,
    Json(body): Json<CriarComentario>,
) -> Result<impl IntoResponse, AppError> {
    // Comments are public — no auth required
    let storage = lock_storage(&state)?;
    let comentario = quilombo_storage::criar_comentario(storage.conn(), &body, None)
        .map_err(AppError::BadRequest)?;

    Ok((StatusCode::CREATED, Json(comentario)))
}

// --- Contact Form ---

async fn contato_handler(
    State(state): State<AppState>,
    Json(body): Json<CriarMensagem>,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state)?;
    let msg = quilombo_storage::criar_mensagem(storage.conn(), &body, None)
        .map_err(AppError::BadRequest)?;

    quilombo_storage::registrar_atividade(
        storage.conn(),
        "criar",
        "contato",
        Some(&msg.id.to_string()),
        None,
        None,
    );

    Ok((StatusCode::CREATED, Json(msg)))
}

// --- Profile Handlers ---

async fn obter_perfil_handler(
    State(state): State<AppState>,
    user_id: UserId,
) -> Result<Json<Usuario>, AppError> {
    let storage = lock_storage(&state)?;
    quilombo_storage::obter_usuario_por_id(storage.conn(), &user_id.0)
        .map(Json)
        .ok_or_else(|| AppError::NotFound("Profile not found".into()))
}

async fn atualizar_perfil_handler(
    State(state): State<AppState>,
    user_id: UserId,
    Json(body): Json<AtualizarPerfil>,
) -> Result<Json<Usuario>, AppError> {
    let storage = lock_storage(&state)?;
    let usuario = quilombo_storage::atualizar_perfil(storage.conn(), &user_id.0, &body)
        .map_err(AppError::NotFound)?;

    Ok(Json(usuario))
}

// --- Message Handlers ---

async fn listar_mensagens_handler(
    State(state): State<AppState>,
    user_id: UserId,
) -> Result<Json<Vec<Mensagem>>, AppError> {
    let storage = lock_storage(&state)?;
    Ok(Json(quilombo_storage::listar_mensagens(
        storage.conn(),
        &user_id.0,
    )))
}

async fn criar_mensagem_handler(
    State(state): State<AppState>,
    user_id: UserId,
    Json(body): Json<CriarMensagem>,
) -> Result<impl IntoResponse, AppError> {
    let storage = lock_storage(&state)?;
    let msg = quilombo_storage::criar_mensagem(storage.conn(), &body, Some(&user_id.0))
        .map_err(AppError::BadRequest)?;

    Ok((StatusCode::CREATED, Json(msg)))
}

// --- Admin Handlers ---

async fn listar_atividades_handler(
    State(state): State<AppState>,
    user_id: UserId,
) -> Result<Json<Vec<Atividade>>, AppError> {
    let storage = lock_storage(&state)?;
    let user = lookup_quilombo_user(&storage, &user_id.0)?;

    if !tem_permissao(&user.papel, "admin:atividades") {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    Ok(Json(quilombo_storage::listar_atividades(
        storage.conn(),
        200,
    )))
}
