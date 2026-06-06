//! CO-361: Typed audit log — `atividades` table + deferred-write helper.
//!
//! Every meaningful mutation (task CRUD, universe CRUD, login/logout) calls
//! `log_atividade()`, which defers the INSERT to a `tokio::spawn` task so it
//! never adds latency to the originating request.
//!
//! Privacy guarantees:
//! - SENSITIVE_KEYS are redacted to `"[REDACTED]"` before write (never plaintext).
//! - IP addresses are hashed with SHA-256 → first 16 hex chars.
//! - User-agents are stored verbatim (already public client info).

use rusqlite::params;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::eda::event::{Event, Visibility};
use crate::server::AppState;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub enum Acao {
    Criar,
    Ler,
    Atualizar,
    Excluir,
    Login,
    Logout,
}

impl Acao {
    pub fn as_str(self) -> &'static str {
        match self {
            Acao::Criar => "criar",
            Acao::Ler => "ler",
            Acao::Atualizar => "atualizar",
            Acao::Excluir => "excluir",
            Acao::Login => "login",
            Acao::Logout => "logout",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Tipo {
    Sucesso,
    Erro,
    Navegacao,
    Sistema,
}

impl Tipo {
    pub fn as_str(self) -> &'static str {
        match self {
            Tipo::Sucesso => "sucesso",
            Tipo::Erro => "erro",
            Tipo::Navegacao => "navegacao",
            Tipo::Sistema => "sistema",
        }
    }
}

// ---------------------------------------------------------------------------
// Struct
// ---------------------------------------------------------------------------

pub struct Atividade {
    pub acao: Acao,
    pub entidade: String,
    pub entidade_id: Option<String>,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
    pub tipo: Tipo,
    pub user_id: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

const SENSITIVE_KEYS: &[&str] = &[
    "password",
    "password_hash",
    "token",
    "api_token",
    "jwt",
    "secret",
    "private_key",
    "ssh_key",
    "session_token",
];

/// Walk a JSON tree and replace any value whose key (case-insensitive) matches
/// SENSITIVE_KEYS with the string `"[REDACTED]"`, at any depth.
pub fn redact(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let sensitive = SENSITIVE_KEYS.iter().any(|s| k.to_ascii_lowercase() == *s);
                out.insert(
                    k,
                    if sensitive {
                        serde_json::Value::String("[REDACTED]".into())
                    } else {
                        redact(v)
                    },
                );
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(redact).collect())
        }
        other => other,
    }
}

// ---------------------------------------------------------------------------
// IP hashing
// ---------------------------------------------------------------------------

/// SHA-256 hash of the IP, truncated to the first 16 hex chars.
/// Sufficient for "same visitor" cohorting; not reversible.
pub fn sha256_short(input: &str) -> String {
    let hash = Sha256::digest(input.as_bytes());
    format!("{:x}", hash)[..16].to_string()
}

// ---------------------------------------------------------------------------
// Deferred write
// ---------------------------------------------------------------------------

/// Enqueue an audit-log write on a background Tokio task.
///
/// Never blocks the caller. Uses `try_lock()` to avoid holding `Mutex<Storage>`
/// across an await point; warns and skips if contended at that instant.
///
/// CO-380: also publishes to the EDA bus so all subscribers receive the event.
pub fn log_atividade(state: AppState, atividade: Atividade) {
    tokio::spawn(async move {
        let conteudo = redact(json!({"before": atividade.before, "after": atividade.after}));
        let conteudo_str = conteudo.to_string();
        let ip_hash = atividade.ip.as_deref().map(sha256_short);
        let versao = env!("CARGO_PKG_VERSION");

        // CO-380: publish to EDA bus BEFORE the SQL write so the event
        // is in-flight even if the write is briefly contended.
        let event_type = format!("atividade.{}", atividade.acao.as_str());
        let payload = json!({
            "entidade": atividade.entidade,
            "entidade_id": atividade.entidade_id,
            "tipo": atividade.tipo.as_str(),
            "conteudo": conteudo,
            "versao_app": versao,
        });
        state.core.eda_bus.publish(Event::new(
            event_type,
            None,
            atividade.user_id.clone(),
            payload,
            Visibility::System,
        ));

        let Some(storage) = state.core.storage.try_lock() else {
            tracing::warn!("log_atividade: storage lock contended — audit event dropped");
            return;
        };

        if let Err(e) = storage.conn().execute(
            "INSERT INTO atividades \
             (acao, entidade, entidade_id, conteudo, tipo, user_id, ip_hash, user_agent, versao_app) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                atividade.acao.as_str(),
                atividade.entidade,
                atividade.entidade_id,
                conteudo_str,
                atividade.tipo.as_str(),
                atividade.user_id,
                ip_hash,
                atividade.user_agent,
                versao,
            ],
        ) {
            tracing::warn!("log_atividade: INSERT failed: {e}");
        }
    });
}

// ---------------------------------------------------------------------------
// 180-day retention
// ---------------------------------------------------------------------------

/// Background task: delete `atividades` rows older than 180 days.
/// Sleeps 24 h between runs; starts on its first tick so a fresh deploy
/// immediately prunes any accumulated backlog.
pub async fn retention_task(state: AppState) {
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(24 * 3600)).await;
        let storage = state.core.storage.lock();
        match storage.conn().execute(
            "DELETE FROM atividades WHERE criado_em < datetime('now', '-180 days')",
            [],
        ) {
            Ok(n) if n > 0 => {
                tracing::info!("atividades retention: purged {n} row(s) older than 180 days")
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("atividades retention: DELETE failed: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_short_is_16_chars() {
        let h = sha256_short("127.0.0.1");
        assert_eq!(h.len(), 16, "expected 16 hex chars, got {h}");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn redact_strips_top_level_sensitive_key() {
        let v = json!({"password": "s3cr3t", "name": "alice"});
        let r = redact(v);
        assert_eq!(r["password"], "[REDACTED]");
        assert_eq!(r["name"], "alice");
    }

    #[test]
    fn redact_strips_nested_sensitive_key() {
        let v = json!({"user": {"password_hash": "abc123", "email": "a@b.com"}});
        let r = redact(v);
        assert_eq!(r["user"]["password_hash"], "[REDACTED]");
        assert_eq!(r["user"]["email"], "a@b.com");
    }

    #[test]
    fn redact_case_insensitive() {
        let v = json!({"TOKEN": "tok123", "Api_Token": "api456"});
        let r = redact(v);
        assert_eq!(r["TOKEN"], "[REDACTED]");
        assert_eq!(r["Api_Token"], "[REDACTED]");
    }

    #[test]
    fn redact_arrays_are_walked() {
        let v = json!([{"secret": "x"}, {"ok": "y"}]);
        let r = redact(v);
        assert_eq!(r[0]["secret"], "[REDACTED]");
        assert_eq!(r[1]["ok"], "y");
    }
}
