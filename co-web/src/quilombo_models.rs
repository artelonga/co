use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// --- Usuario ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usuario {
    pub id: String,
    pub usuario: String,
    pub nome: String,
    pub papel: Papel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foto_url: Option<String>,
    pub criado_em: DateTime<Utc>,
    pub atualizado_em: DateTime<Utc>,
}

/// Public-facing member profile (excludes sensitive fields)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membro {
    pub id: String,
    pub usuario: String,
    pub nome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foto_url: Option<String>,
    pub criado_em: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Papel {
    Admin,
    Editor,
    Membro,
}

impl std::fmt::Display for Papel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Papel::Admin => write!(f, "admin"),
            Papel::Editor => write!(f, "editor"),
            Papel::Membro => write!(f, "membro"),
        }
    }
}

impl std::str::FromStr for Papel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "admin" => Ok(Papel::Admin),
            "editor" => Ok(Papel::Editor),
            "membro" => Ok(Papel::Membro),
            _ => Err(format!("Invalid papel: {s}")),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CriarUsuario {
    pub usuario: String,
    pub nome: String,
    pub senha: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginUsuario {
    pub usuario: String,
    pub senha: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub usuario: UsuarioResumo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsuarioResumo {
    pub id: String,
    pub usuario: String,
    pub nome: String,
    pub papel: Papel,
}

#[derive(Debug, Deserialize)]
pub struct AtualizarPerfil {
    pub nome: Option<String>,
    pub bio: Option<String>,
    pub foto_url: Option<String>,
}

// --- Evento ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evento {
    pub id: i64,
    pub titulo: String,
    pub data: String,
    pub hora: String,
    pub local: String,
    pub descricao_md: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criado_por: Option<String>,
    pub criado_em: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CriarEvento {
    pub titulo: String,
    pub data: String,
    pub hora: String,
    pub local: String,
    #[serde(default)]
    pub descricao_md: String,
}

#[derive(Debug, Deserialize)]
pub struct AtualizarEvento {
    pub titulo: Option<String>,
    pub data: Option<String>,
    pub hora: Option<String>,
    pub local: Option<String>,
    pub descricao_md: Option<String>,
}

// --- Missao ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Missao {
    pub id: i64,
    pub titulo: String,
    pub descricao: String,
    pub objetivo: String,
    pub status: StatusMissao,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criado_por: Option<String>,
    pub criado_em: DateTime<Utc>,
    pub atualizado_em: DateTime<Utc>,
    #[serde(default)]
    pub participantes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StatusMissao {
    Aberta,
    EmAndamento,
    Concluida,
    Cancelada,
}

impl std::fmt::Display for StatusMissao {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatusMissao::Aberta => write!(f, "aberta"),
            StatusMissao::EmAndamento => write!(f, "em_andamento"),
            StatusMissao::Concluida => write!(f, "concluida"),
            StatusMissao::Cancelada => write!(f, "cancelada"),
        }
    }
}

impl std::str::FromStr for StatusMissao {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "aberta" => Ok(StatusMissao::Aberta),
            "em_andamento" => Ok(StatusMissao::EmAndamento),
            "concluida" => Ok(StatusMissao::Concluida),
            "cancelada" => Ok(StatusMissao::Cancelada),
            _ => Err(format!("Invalid status: {s}")),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CriarMissao {
    pub titulo: String,
    #[serde(default)]
    pub descricao: String,
    #[serde(default)]
    pub objetivo: String,
}

#[derive(Debug, Deserialize)]
pub struct AtualizarMissao {
    pub titulo: Option<String>,
    pub descricao: Option<String>,
    pub objetivo: Option<String>,
    pub status: Option<StatusMissao>,
}

// --- Participacao ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participacao {
    pub id: i64,
    pub missao_id: i64,
    pub usuario_id: String,
    pub status: StatusParticipacao,
    pub criado_em: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StatusParticipacao {
    Pendente,
    Aprovado,
    Recusado,
}

impl std::fmt::Display for StatusParticipacao {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatusParticipacao::Pendente => write!(f, "pendente"),
            StatusParticipacao::Aprovado => write!(f, "aprovado"),
            StatusParticipacao::Recusado => write!(f, "recusado"),
        }
    }
}

impl std::str::FromStr for StatusParticipacao {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pendente" => Ok(StatusParticipacao::Pendente),
            "aprovado" => Ok(StatusParticipacao::Aprovado),
            "recusado" => Ok(StatusParticipacao::Recusado),
            _ => Err(format!("Invalid status: {s}")),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AtualizarParticipacao {
    pub status: StatusParticipacao,
}

// --- Comentario ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comentario {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publicacao_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evento_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nome_exibicao: Option<String>,
    pub conteudo: String,
    pub anonimo: bool,
    pub criado_em: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CriarComentario {
    #[serde(default)]
    pub publicacao_slug: Option<String>,
    #[serde(default)]
    pub evento_id: Option<i64>,
    #[serde(default)]
    pub nome_exibicao: Option<String>,
    pub conteudo: String,
    #[serde(default = "default_true")]
    pub anonimo: bool,
}

fn default_true() -> bool {
    true
}

// --- Mensagem ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mensagem {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remetente_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destinatario_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missao_id: Option<i64>,
    pub assunto: String,
    pub conteudo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nome_remetente: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_remetente: Option<String>,
    pub lida: bool,
    pub criado_em: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CriarMensagem {
    #[serde(default)]
    pub destinatario_id: Option<String>,
    #[serde(default)]
    pub missao_id: Option<i64>,
    #[serde(default)]
    pub assunto: String,
    pub conteudo: String,
    #[serde(default)]
    pub nome_remetente: Option<String>,
    #[serde(default)]
    pub email_remetente: Option<String>,
}

// --- Atividade ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Atividade {
    pub id: i64,
    pub acao: String,
    pub entidade: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entidade_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usuario_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conteudo: Option<String>,
    pub criado_em: DateTime<Utc>,
}

// --- Perfil (user-editable overrides) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Perfil {
    pub usuario_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nome_exibicao: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foto_url: Option<String>,
    pub atualizado_em: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct AtualizarPerfilUnificado {
    pub nome_exibicao: Option<String>,
    pub bio: Option<String>,
    pub foto_url: Option<String>,
}

// --- Telemetria ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Telemetria {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visitante_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usuario_id: Option<String>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duracao_ms: Option<i64>,
    pub criado_em: DateTime<Utc>,
}

// --- Merged Member Profile (markdown + SQLite) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfilCompleto {
    pub id: String,
    pub usuario: String,
    pub nome: String,
    pub papel: Papel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foto_url: Option<String>,
    pub tags: Vec<String>,
    pub criado_em: DateTime<Utc>,
}

// --- Data Export ---

#[derive(Debug, Serialize)]
pub struct ExportacaoDados {
    pub perfil: PerfilCompleto,
    pub comentarios: Vec<Comentario>,
    pub mensagens: Vec<Mensagem>,
    pub telemetria: Vec<Telemetria>,
    pub historico: Vec<Atividade>,
    pub exportado_em: DateTime<Utc>,
}

// --- Query Params ---

#[derive(Debug, Deserialize)]
pub struct ComentarioQuery {
    #[serde(default)]
    pub relato: Option<String>,
    #[serde(default)]
    pub evento: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PaginacaoQuery {
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_limit() -> u64 {
    50
}
