//! Gestão (admin) routes for content CRUD via GitHub-authenticated API.
//!
//! All routes require a valid GitHub PAT from an allowed admin.
//! Content is written directly to the universe filesystem (markdown files).

use axum::Router;
use axum::extract::{Json, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::error::AppError;
use crate::github_auth::{self, GitHubAdmin};
use crate::iceberg;
use crate::server::AppState;

// ---- Request / Response types ----

#[derive(Debug, Deserialize)]
pub struct PublicarRequest {
    pub caminho: String,
    pub conteudo: String,
}

#[derive(Debug, Serialize)]
pub struct PublicarResponse {
    pub caminho: String,
    pub tipo: String,
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct ValidarResponse {
    pub valido: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub erros: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub avisos: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CriarConteudo {
    pub id: String,
    pub titulo: String,
    #[serde(default)]
    pub descricao: String,
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub conteudo: String,
    // Evento-specific
    #[serde(default)]
    pub hora: String,
    #[serde(default)]
    pub local: String,
    // Membro-specific
    #[serde(default)]
    pub nome: String,
    #[serde(default)]
    pub papel: String,
    // Quadro-specific
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub prioridade: String,
    #[serde(default)]
    pub objetivo: String,
    // Relato-specific
    #[serde(default)]
    pub publicado: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct AtualizarConteudo {
    pub titulo: Option<String>,
    pub descricao: Option<String>,
    pub data: Option<String>,
    pub tags: Option<Vec<String>>,
    pub conteudo: Option<String>,
    pub hora: Option<String>,
    pub local: Option<String>,
    pub nome: Option<String>,
    pub papel: Option<String>,
    pub status: Option<String>,
    pub prioridade: Option<String>,
    pub objetivo: Option<String>,
    pub publicado: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ConteudoResumo {
    pub id: String,
    pub tipo: String,
    pub titulo: String,
    pub caminho: String,
}

// CO-137: schema diagnostic types

#[derive(Debug, Serialize)]
pub struct SchemaColumn {
    pub cid: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub col_type: String,
    pub notnull: i64,
    pub dflt_value: Option<String>,
    pub pk: i64,
}

#[derive(Debug, Serialize)]
pub struct SchemaCheckResponse {
    pub universes_columns: Vec<SchemaColumn>,
    pub schema_versions: Vec<i64>,
}

// ---- Router ----

pub fn router() -> Router<AppState> {
    Router::new()
        // CO-137: diagnostic endpoint — confirms prod schema state for universes table
        .route("/_schema_check", get(schema_check_handler))
        // CO-406: list universes the pool currently marks unavailable, and an
        // admin reopen to recover one after fixing the environment (no restart).
        .route(
            "/universos/indisponiveis",
            get(universes_unavailable_handler),
        )
        .route("/universos/{key}/reabrir", post(reopen_universe_handler))
        // Validate & publish
        .route("/validar", post(validar_handler))
        .route("/publicar", post(publicar_handler))
        // Relatos CRUD
        .route(
            "/relatos",
            get(listar_handler::<RelatoDir>).post(criar_handler::<RelatoDir>),
        )
        .route(
            "/relatos/{id}",
            put(atualizar_handler::<RelatoDir>).delete(excluir_handler::<RelatoDir>),
        )
        // Eventos CRUD
        .route(
            "/eventos",
            get(listar_handler::<EventoDir>).post(criar_handler::<EventoDir>),
        )
        .route(
            "/eventos/{id}",
            put(atualizar_handler::<EventoDir>).delete(excluir_handler::<EventoDir>),
        )
        // Membros CRUD
        .route(
            "/membros",
            get(listar_handler::<MembroDir>).post(criar_handler::<MembroDir>),
        )
        .route(
            "/membros/{id}",
            put(atualizar_handler::<MembroDir>).delete(excluir_handler::<MembroDir>),
        )
        // Quadro CRUD
        .route(
            "/quadro",
            get(listar_handler::<QuadroDir>).post(criar_handler::<QuadroDir>),
        )
        .route(
            "/quadro/{id}",
            put(atualizar_handler::<QuadroDir>).delete(excluir_handler::<QuadroDir>),
        )
        // Manifest
        .route("/manifesto", get(manifesto_handler))
        .route(
            "/manifesto/reconstruir",
            post(reconstruir_manifesto_handler),
        )
        // Note: /atividades is now provided by resumo_routes (email admin auth, CO-360)
        .route("/schema-status", get(schema_status_handler))
        // CO-378: analytics resumo (includes private paths).
        // Mounted at /analytics/resumo — the bare /resumo at this prefix belongs
        // to the CO-360 dashboard endpoint in resumo_routes.
        .route("/analytics/resumo", get(resumo_handler))
        // GitHub auth on all routes
        .layer(middleware::from_fn(github_auth::require_github_admin))
}

/// Serve the `/gestao` admin SPA page (no GitHub auth — auth is handled
/// client-side when making API calls with the PAT).
pub async fn serve_gestao_page(headers: axum::http::HeaderMap) -> axum::response::Response {
    use crate::admin::admin_routes::{check_admin_email, extract_claims};

    match extract_claims(&headers) {
        Err(_) => axum::response::Redirect::to("/").into_response(),
        Ok(claims) => {
            if !check_admin_email(&claims.email) {
                (
                    StatusCode::FORBIDDEN,
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    "<html><body><h1>403 Proibido</h1><p>Acesso restrito a administradores.</p></body></html>",
                )
                    .into_response()
            } else {
                (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (header::CACHE_CONTROL, "no-store"),
                    ],
                    GESTAO_PAGE_HTML,
                )
                    .into_response()
            }
        }
    }
}

const GESTAO_PAGE_HTML: &str = include_str!("../../static/variants/a/gestao.html");

// ---- Content directory trait ----

trait ContentDir: Send + Sync + 'static {
    const DIR_NAME: &'static str;
    const TIPO: &'static str;

    fn gerar_frontmatter(body: &CriarConteudo) -> String;
    fn campos_obrigatorios() -> &'static [&'static str];
}

struct RelatoDir;
struct EventoDir;
struct MembroDir;
struct QuadroDir;

impl ContentDir for RelatoDir {
    const DIR_NAME: &'static str = "relatos";
    const TIPO: &'static str = "relato";

    fn gerar_frontmatter(b: &CriarConteudo) -> String {
        let tags = format_tags(&b.tags);
        let publicado = b.publicado.unwrap_or(false);
        format!(
            "---\ntype: relato\nid: {}\ntitulo: {}\ndescricao: \"{}\"\ndata: \"{}\"\ntags: {}\npublicado: {}\nslug: {}\n---\n",
            b.id, b.titulo, b.descricao, b.data, tags, publicado, b.id
        )
    }

    fn campos_obrigatorios() -> &'static [&'static str] {
        &["titulo"]
    }
}

impl ContentDir for EventoDir {
    const DIR_NAME: &'static str = "eventos";
    const TIPO: &'static str = "evento";

    fn gerar_frontmatter(b: &CriarConteudo) -> String {
        let tags = format_tags(&b.tags);
        format!(
            "---\ntype: evento\nid: {}\ntitulo: {}\ndata: \"{}\"\nhora: \"{}\"\nlocal: {}\ntags: {}\n---\n",
            b.id, b.titulo, b.data, b.hora, b.local, tags
        )
    }

    fn campos_obrigatorios() -> &'static [&'static str] {
        &["titulo", "data", "hora", "local"]
    }
}

impl ContentDir for MembroDir {
    const DIR_NAME: &'static str = "membros";
    const TIPO: &'static str = "membro";

    fn gerar_frontmatter(b: &CriarConteudo) -> String {
        let nome = if b.nome.is_empty() {
            &b.titulo
        } else {
            &b.nome
        };
        let papel = if b.papel.is_empty() {
            "membro"
        } else {
            &b.papel
        };
        let tags = format_tags(&b.tags);
        format!(
            "---\ntype: membro\nid: {}\nnome: {}\npapel: {}\ntags: {}\n---\n",
            b.id, nome, papel, tags
        )
    }

    fn campos_obrigatorios() -> &'static [&'static str] {
        &["titulo"]
    }
}

impl ContentDir for QuadroDir {
    const DIR_NAME: &'static str = "quadro";
    const TIPO: &'static str = "missao";

    fn gerar_frontmatter(b: &CriarConteudo) -> String {
        let status = if b.status.is_empty() {
            "aberta"
        } else {
            &b.status
        };
        let tags = format_tags(&b.tags);
        format!(
            "---\ntitulo: {}\nstatus: {}\nprioridade: {}\netiquetas: {}\ncriado: \"{}\"\n---\n",
            b.titulo, status, b.prioridade, tags, b.data
        )
    }

    fn campos_obrigatorios() -> &'static [&'static str] {
        &["titulo"]
    }
}

fn format_tags(tags: &[String]) -> String {
    if tags.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", tags.join(", "))
    }
}

// ---- Validation ----

fn validar_conteudo(conteudo: &str) -> ValidarResponse {
    let mut erros = Vec::new();
    let mut avisos = Vec::new();

    let trimmed = conteudo.trim_start();
    if !trimmed.starts_with("---") {
        erros.push("frontmatter ausente (arquivo deve começar com ---)".to_string());
        return ValidarResponse {
            valido: false,
            erros,
            avisos,
        };
    }

    let after_opening = &trimmed[3..];
    let end_idx = match after_opening.find("\n---") {
        Some(i) => i,
        None => {
            erros.push("frontmatter não fechado (falta --- de fechamento)".to_string());
            return ValidarResponse {
                valido: false,
                erros,
                avisos,
            };
        }
    };

    let fm_block = &after_opening[..end_idx];

    // Parse as YAML
    let fm: Result<HashMap<String, serde_yaml::Value>, _> = serde_yaml::from_str(fm_block);
    let fm = match fm {
        Ok(fm) => fm,
        Err(e) => {
            erros.push(format!("YAML invalido: {e}"));
            return ValidarResponse {
                valido: false,
                erros,
                avisos,
            };
        }
    };

    // Check type field
    let tipo = fm.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if tipo.is_empty() {
        // quadro items might not have type field (they use status)
        if !fm.contains_key("status") {
            avisos.push("campo 'type' ausente (recomendado)".to_string());
        }
    } else {
        let known_types = ["relato", "evento", "membro", "missao", "pagina", "nota"];
        if !known_types.contains(&tipo) {
            erros.push(format!("tipo '{}' desconhecido", tipo));
        }
    }

    // Type-specific required fields
    match tipo {
        "relato" => {
            if !fm.contains_key("titulo") {
                erros.push("campo 'titulo' obrigatorio para tipo 'relato'".to_string());
            }
        }
        "evento" => {
            for campo in &["titulo", "data", "hora", "local"] {
                if !fm.contains_key(*campo) {
                    erros.push(format!("campo '{}' obrigatorio para tipo 'evento'", campo));
                }
            }
        }
        "membro" => {
            if !fm.contains_key("nome") && !fm.contains_key("titulo") {
                erros.push("campo 'nome' obrigatorio para tipo 'membro'".to_string());
            }
        }
        _ => {}
    }

    // Date format check
    if let Some(data) = fm.get("data").and_then(|v| v.as_str()) {
        let data = data.trim().trim_matches('"');
        if !data.is_empty() && chrono::NaiveDate::parse_from_str(data, "%Y-%m-%d").is_err() {
            erros.push(format!(
                "campo 'data' formato invalido: '{}' (use YYYY-MM-DD)",
                data
            ));
        }
    }

    // ID character check
    if let Some(id) = fm.get("id").and_then(|v| v.as_str())
        && co::validate_id(id).is_err()
    {
        erros.push(format!("id contem caracteres invalidos: '{id}'"));
    }

    ValidarResponse {
        valido: erros.is_empty(),
        erros,
        avisos,
    }
}

fn universo_dir(state: &AppState) -> PathBuf {
    PathBuf::from(&state.core.config.universo_dir)
}

// ---- Handlers ----

async fn validar_handler(
    _admin: GitHubAdmin,
    Json(body): Json<PublicarRequest>,
) -> Json<ValidarResponse> {
    Json(validar_conteudo(&body.conteudo))
}

async fn publicar_handler(
    State(state): State<AppState>,
    admin: GitHubAdmin,
    Json(body): Json<PublicarRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Validate first
    let validation = validar_conteudo(&body.conteudo);
    if !validation.valido {
        return Err(AppError::BadRequest(
            serde_json::to_string(&validation).unwrap_or_else(|_| "Validation failed".into()),
        ));
    }

    // Security: prevent path traversal
    let caminho = body.caminho.trim_start_matches('/');
    if caminho.contains("..") || caminho.starts_with('.') {
        return Err(AppError::BadRequest("caminho fora do universo".into()));
    }

    let allowed_prefixes = ["relatos/", "eventos/", "membros/", "quadro/", "jardim/"];
    if !allowed_prefixes.iter().any(|p| caminho.starts_with(p)) {
        return Err(AppError::BadRequest(
            "caminho deve começar com relatos/, eventos/, membros/, quadro/ ou jardim/".into(),
        ));
    }

    let file_path = universo_dir(&state).join(caminho);

    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| AppError::Internal(format!("Failed to create directory: {e}")))?;
    }

    fs::write(&file_path, &body.conteudo)
        .map_err(|e| AppError::Internal(format!("Failed to write file: {e}")))?;

    tracing::info!("Publicado por {}: {}", admin.0, caminho);

    // Extract type and id from frontmatter
    let (fm, _) = parse_fm_simple(&body.conteudo);
    let tipo = fm.get("type").cloned().unwrap_or_default();
    let id = fm.get("id").cloned().unwrap_or_else(|| {
        file_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    });

    Ok((
        StatusCode::CREATED,
        Json(PublicarResponse {
            caminho: caminho.to_string(),
            tipo,
            id,
        }),
    ))
}

async fn listar_handler<D: ContentDir>(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
) -> Result<Json<Vec<ConteudoResumo>>, AppError> {
    let dir = universo_dir(&state).join(D::DIR_NAME);
    let mut items = Vec::new();

    let entries = fs::read_dir(&dir).into_iter().flatten().flatten();
    for entry in entries {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let (fm, _) = parse_fm_simple(&content);
        let id = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let titulo = fm
            .get("titulo")
            .or_else(|| fm.get("title"))
            .cloned()
            .unwrap_or_else(|| id.clone());

        items.push(ConteudoResumo {
            id,
            tipo: D::TIPO.to_string(),
            titulo,
            caminho: format!(
                "{}/{}",
                D::DIR_NAME,
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
        });
    }

    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(items))
}

async fn criar_handler<D: ContentDir>(
    State(state): State<AppState>,
    admin: GitHubAdmin,
    Json(body): Json<CriarConteudo>,
) -> Result<impl IntoResponse, AppError> {
    // Validate ID
    if body.id.is_empty() {
        return Err(AppError::BadRequest("campo 'id' obrigatorio".into()));
    }
    if let Err(invalid) = co::validate_id(&body.id) {
        return Err(AppError::BadRequest(format!(
            "id contem caracteres invalidos: {:?}",
            invalid
        )));
    }

    // Validate required fields
    for campo in D::campos_obrigatorios() {
        let val = match *campo {
            "titulo" => &body.titulo,
            "data" => &body.data,
            "hora" => &body.hora,
            "local" => &body.local,
            _ => continue,
        };
        if val.is_empty() {
            return Err(AppError::BadRequest(format!(
                "campo '{}' obrigatorio para tipo '{}'",
                campo,
                D::TIPO
            )));
        }
    }

    let file_path = universo_dir(&state)
        .join(D::DIR_NAME)
        .join(format!("{}.md", body.id));

    if file_path.exists() {
        return Err(AppError::Conflict(format!(
            "{} '{}' ja existe",
            D::TIPO,
            body.id
        )));
    }

    // Generate markdown
    let frontmatter = D::gerar_frontmatter(&body);
    let full_content = if body.conteudo.is_empty() {
        frontmatter
    } else {
        format!("{}\n{}\n", frontmatter, body.conteudo)
    };

    // Ensure directory exists
    let dir = universo_dir(&state).join(D::DIR_NAME);
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::Internal(format!("Failed to create directory: {e}")))?;

    fs::write(&file_path, &full_content)
        .map_err(|e| AppError::Internal(format!("Failed to write file: {e}")))?;

    tracing::info!("Criado {} '{}' por {}", D::TIPO, body.id, admin.0);

    Ok((
        StatusCode::CREATED,
        Json(ConteudoResumo {
            id: body.id.clone(),
            tipo: D::TIPO.to_string(),
            titulo: body.titulo,
            caminho: format!("{}/{}.md", D::DIR_NAME, body.id),
        }),
    ))
}

async fn atualizar_handler<D: ContentDir>(
    State(state): State<AppState>,
    admin: GitHubAdmin,
    Path(id): Path<String>,
    Json(body): Json<AtualizarConteudo>,
) -> Result<Json<ConteudoResumo>, AppError> {
    let file_path = universo_dir(&state)
        .join(D::DIR_NAME)
        .join(format!("{id}.md"));

    if !file_path.exists() {
        return Err(AppError::NotFound(format!(
            "{} '{}' nao encontrado",
            D::TIPO,
            id
        )));
    }

    let content = fs::read_to_string(&file_path)
        .map_err(|e| AppError::Internal(format!("Failed to read file: {e}")))?;

    let (mut fm, old_body) = parse_fm_simple(&content);

    // Apply updates to frontmatter
    if let Some(v) = &body.titulo {
        fm.insert("titulo".into(), v.clone());
    }
    if let Some(v) = &body.descricao {
        fm.insert("descricao".into(), v.clone());
    }
    if let Some(v) = &body.data {
        fm.insert("data".into(), format!("\"{}\"", v));
    }
    if let Some(v) = &body.hora {
        fm.insert("hora".into(), format!("\"{}\"", v));
    }
    if let Some(v) = &body.local {
        fm.insert("local".into(), v.clone());
    }
    if let Some(v) = &body.nome {
        fm.insert("nome".into(), v.clone());
    }
    if let Some(v) = &body.papel {
        fm.insert("papel".into(), v.clone());
    }
    if let Some(v) = &body.status {
        fm.insert("status".into(), v.clone());
    }
    if let Some(v) = &body.prioridade {
        fm.insert("prioridade".into(), v.clone());
    }
    if let Some(v) = &body.objetivo {
        fm.insert("objetivo".into(), v.clone());
    }
    if let Some(v) = &body.publicado {
        fm.insert("publicado".into(), v.to_string());
    }
    if let Some(tags) = &body.tags {
        fm.insert("tags".into(), format_tags(tags));
    }

    // Rebuild file
    let new_body = body.conteudo.as_deref().unwrap_or(&old_body);
    let new_content = rebuild_md(&fm, new_body);

    fs::write(&file_path, &new_content)
        .map_err(|e| AppError::Internal(format!("Failed to write file: {e}")))?;

    tracing::info!("Atualizado {} '{}' por {}", D::TIPO, id, admin.0);

    let titulo = fm
        .get("titulo")
        .or_else(|| fm.get("title"))
        .cloned()
        .unwrap_or_else(|| id.clone());

    Ok(Json(ConteudoResumo {
        id: id.clone(),
        tipo: D::TIPO.to_string(),
        titulo,
        caminho: format!("{}/{}.md", D::DIR_NAME, id),
    }))
}

async fn excluir_handler<D: ContentDir>(
    State(state): State<AppState>,
    admin: GitHubAdmin,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let file_path = universo_dir(&state)
        .join(D::DIR_NAME)
        .join(format!("{id}.md"));

    if !file_path.exists() {
        return Err(AppError::NotFound(format!(
            "{} '{}' nao encontrado",
            D::TIPO,
            id
        )));
    }

    fs::remove_file(&file_path)
        .map_err(|e| AppError::Internal(format!("Failed to delete file: {e}")))?;

    tracing::info!("Excluido {} '{}' por {}", D::TIPO, id, admin.0);

    Ok(StatusCode::NO_CONTENT)
}

// ---- Manifest ----

async fn manifesto_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
) -> Result<Json<iceberg::Manifest>, AppError> {
    let manifest = iceberg::build_manifest(&universo_dir(&state), 1);
    Ok(Json(manifest))
}

async fn reconstruir_manifesto_handler(
    State(state): State<AppState>,
    admin: GitHubAdmin,
) -> Result<Json<iceberg::Manifest>, AppError> {
    let manifest = iceberg::build_manifest(&universo_dir(&state), 1);
    tracing::info!(
        "Manifesto reconstruido por {} ({} entradas)",
        admin.0,
        manifest.entries.len()
    );
    Ok(Json(manifest))
}

// ---- Helpers ----

fn parse_fm_simple(content: &str) -> (HashMap<String, String>, String) {
    let mut fm = HashMap::new();
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (fm, content.to_string());
    }
    let after_opening = &trimmed[3..];
    if let Some(end_idx) = after_opening.find("\n---") {
        let fm_block = &after_opening[..end_idx];
        for line in fm_block.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((key, value)) = line.split_once(':') {
                fm.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
        let body_start = 3 + end_idx + 4;
        let body = if body_start < trimmed.len() {
            trimmed[body_start..].trim_start_matches('\n').to_string()
        } else {
            String::new()
        };
        (fm, body)
    } else {
        (fm, content.to_string())
    }
}

fn rebuild_md(fm: &HashMap<String, String>, body: &str) -> String {
    let mut lines: Vec<String> = fm.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
    lines.sort();
    let frontmatter = lines.join("\n");
    if body.is_empty() {
        format!("---\n{}\n---\n", frontmatter)
    } else {
        format!("---\n{}\n---\n\n{}\n", frontmatter, body)
    }
}

// CO-137: diagnostic endpoint — returns universes column list + schema_version rows.
// Used to confirm prod schema state after the parent_key migration incident.
// Accessible at GET /api/v1/gestao/_schema_check (requires GitHub admin auth).
async fn schema_check_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
) -> Result<Json<SchemaCheckResponse>, AppError> {
    let storage = state.core.storage.lock();
    let conn = storage.conn();

    let mut stmt = conn
        .prepare(
            "SELECT cid, name, type, \"notnull\", dflt_value, pk \
             FROM pragma_table_info('universes') ORDER BY cid",
        )
        .map_err(|e| AppError::Internal(format!("pragma_table_info: {e}")))?;

    let universes_columns: Vec<SchemaColumn> = stmt
        .query_map([], |row| {
            Ok(SchemaColumn {
                cid: row.get(0)?,
                name: row.get(1)?,
                col_type: row.get(2)?,
                notnull: row.get(3)?,
                dflt_value: row.get(4)?,
                pk: row.get(5)?,
            })
        })
        .map_err(|e| AppError::Internal(format!("columns query_map: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    let mut stmt = conn
        .prepare("SELECT version FROM schema_version ORDER BY version")
        .map_err(|e| AppError::Internal(format!("schema_version query: {e}")))?;

    let schema_versions: Vec<i64> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| AppError::Internal(format!("versions query_map: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(SchemaCheckResponse {
        universes_columns,
        schema_versions,
    }))
}

// ---- CO-406: per-universe pool availability ----

#[derive(Debug, Serialize)]
pub struct UnavailableUniverse {
    pub universe: String,
    pub stage: String,
    pub reason: String,
}

/// GET /gestao/universos/indisponiveis — list universes the pool currently
/// marks unavailable (CO-406). Empty list means every universe is healthy.
async fn universes_unavailable_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
) -> Result<Json<Vec<UnavailableUniverse>>, AppError> {
    let pool = state.core.storage.lock().universe_pool().clone();
    let list = pool
        .unavailable_universes()
        .into_iter()
        .map(|e| UnavailableUniverse {
            universe: e.universe,
            stage: e.stage.to_string(),
            reason: e.reason,
        })
        .collect();
    Ok(Json(list))
}

/// POST /gestao/universos/{key}/reabrir — admin reopen (CO-406). Drops the
/// cached connection + unavailable mark and re-attempts the open, recovering a
/// universe after the environment failure is fixed, without a machine restart.
async fn reopen_universe_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
    Path(key): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let pool = state.core.storage.lock().universe_pool().clone();
    match pool.reopen(&key) {
        Ok(()) => Ok(Json(serde_json::json!({ "reopened": key, "ok": true }))),
        Err(e) => Err(AppError::from(e)),
    }
}

// ---- CO-361: schema version status ----

#[derive(Debug, Serialize)]
pub struct SchemaStatusResponse {
    /// Highest version number currently in `schema_version`.
    pub schema_versao: i64,
    /// App version compiled into the binary.
    pub app_versao: &'static str,
    /// True when schema_versao == EXPECTED_SCHEMA_VERSION (no drift).
    pub ok: bool,
    /// Most recent rows from schema_versoes (newest first).
    pub historico: Vec<SchemaVersaoRow>,
}

#[derive(Debug, Serialize)]
pub struct SchemaVersaoRow {
    pub versao: i64,
    pub descricao: String,
    pub versao_app: String,
    pub applied_at: String,
}

/// The migration version this binary expects to find after startup.
const EXPECTED_SCHEMA_VERSION: i64 = 59;

// ---- CO-378: analytics resumo (admin view with private paths) ----

#[derive(Debug, Serialize)]
pub struct ResumoTopPage {
    pub path: String,
    pub views: i64,
    pub visitors: i64,
    pub private: bool,
}

#[derive(Debug, Serialize)]
pub struct ResumoResponse {
    pub as_of: String,
    pub window_days: u32,
    pub views: i64,
    pub visitors: i64,
    pub top_pages: Vec<ResumoTopPage>,
    pub private_total_views: i64,
}

/// GET /api/v1/gestao/analytics/resumo?days=N — analytics resumo with private-path visibility.
///
/// Requires GitHub admin auth. Calls query_universe_summary with include_private=true
/// and logs an atividade for auditability.
async fn resumo_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<ResumoResponse>, AppError> {
    let days: u32 = params
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .clamp(1, 365);

    let summary = {
        let storage = state.core.storage.lock();
        crate::admin::analytics_public::query_universe_summary(
            storage.conn(),
            "artelonga",
            days,
            true,
        )
    };

    crate::atividade::log_atividade(
        state,
        crate::atividade::Atividade {
            acao: crate::atividade::Acao::Ler,
            entidade: "analytics".to_string(),
            entidade_id: Some("private_path_viewed".to_string()),
            before: None,
            after: Some(serde_json::json!({
                "event": "analytics.private_path_viewed",
                "universe": "artelonga",
                "days": days,
                "via": "gestao/analytics/resumo",
            })),
            tipo: crate::atividade::Tipo::Sistema,
            user_id: None,
            ip: None,
            user_agent: None,
        },
    );

    let private_total_views: i64 = summary
        .top_pages
        .iter()
        .filter(|p| p.private == Some(true))
        .map(|p| p.views)
        .sum();

    let top_pages: Vec<ResumoTopPage> = summary
        .top_pages
        .into_iter()
        .map(|p| ResumoTopPage {
            path: p.path,
            views: p.views,
            visitors: p.visitors,
            private: p.private.unwrap_or(false),
        })
        .collect();

    Ok(Json(ResumoResponse {
        as_of: summary.as_of,
        window_days: days,
        views: summary.views,
        visitors: summary.visitors,
        top_pages,
        private_total_views,
    }))
}

/// GET /api/v1/gestao/schema-status — schema version info for the header strip.
async fn schema_status_handler(
    State(state): State<AppState>,
    _admin: GitHubAdmin,
) -> Result<Json<SchemaStatusResponse>, AppError> {
    let storage = state.core.storage.lock();
    let conn = storage.conn();

    let schema_versao: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .map_err(|e| AppError::Internal(format!("schema_version query: {e}")))?;

    let mut stmt = conn
        .prepare(
            "SELECT versao, descricao, versao_app, applied_at \
             FROM schema_versoes ORDER BY versao DESC LIMIT 20",
        )
        .map_err(|e| AppError::Internal(format!("schema_versoes prepare: {e}")))?;

    let historico: Vec<SchemaVersaoRow> = stmt
        .query_map([], |row| {
            Ok(SchemaVersaoRow {
                versao: row.get(0)?,
                descricao: row.get(1)?,
                versao_app: row.get(2)?,
                applied_at: row.get(3)?,
            })
        })
        .map_err(|e| AppError::Internal(format!("schema_versoes query: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(SchemaStatusResponse {
        schema_versao,
        app_versao: env!("CARGO_PKG_VERSION"),
        ok: schema_versao == EXPECTED_SCHEMA_VERSION,
        historico,
    }))
}
