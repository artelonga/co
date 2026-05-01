use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    Conflict(String),
    BadRequest(String),
    UnprocessableEntity(String),
    Internal(String),
    Unauthorized(String),
    Forbidden(String),
    Gone(String),
    TooManyRequests(String),
    UsageLimitExceeded {
        current: i64,
    },
    /// CO-80: rate limit exceeded — returns 429 with Retry-After header.
    RateLimited {
        retry_after_secs: u64,
    },
    /// CO-80: tier storage or universe quota exceeded — returns 402 with usage details.
    StorageQuotaExceeded {
        used: i64,
        limit: i64,
        tier: String,
    },
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::NotFound(msg) => write!(f, "Not found: {msg}"),
            AppError::Conflict(msg) => write!(f, "Conflict: {msg}"),
            AppError::BadRequest(msg) => write!(f, "Bad request: {msg}"),
            AppError::UnprocessableEntity(msg) => write!(f, "Unprocessable entity: {msg}"),
            AppError::Internal(msg) => write!(f, "Internal error: {msg}"),
            AppError::Unauthorized(msg) => write!(f, "Unauthorized: {msg}"),
            AppError::Forbidden(msg) => write!(f, "Forbidden: {msg}"),
            AppError::Gone(msg) => write!(f, "Gone: {msg}"),
            AppError::TooManyRequests(msg) => write!(f, "Too many requests: {msg}"),
            AppError::UsageLimitExceeded { current } => {
                write!(f, "Usage limit exceeded: {current}/100")
            }
            AppError::RateLimited { retry_after_secs } => {
                write!(f, "Rate limited: retry after {retry_after_secs}s")
            }
            AppError::StorageQuotaExceeded { used, limit, tier } => {
                write!(f, "Storage quota exceeded ({tier}): {used}/{limit}")
            }
        }
    }
}

fn translate_error_pt(error: &str) -> &'static str {
    match error {
        "not_found" => "Não encontrado",
        "conflict" => "Conflito",
        "bad_request" => "Requisição inválida",
        "unprocessable_entity" => "Dados inválidos",
        "internal_error" => "Erro interno do servidor",
        "unauthorized" => "Não autorizado",
        "forbidden" => "Acesso negado",
        "gone" => "Recurso removido",
        "too_many_requests" => "Muitas requisições. Tente novamente em instantes.",
        _ => "Erro",
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if let AppError::UsageLimitExceeded { current } = self {
            let body = json!({
                "error": "usage_limit",
                "message": "Crie uma conta para continuar",
                "message_en": "Create an account to continue",
                "current": current,
                "limit": 100,
            });
            return (StatusCode::PAYMENT_REQUIRED, axum::Json(body)).into_response();
        }

        if let AppError::StorageQuotaExceeded {
            used,
            limit,
            ref tier,
        } = self
        {
            let body = json!({
                "error": "quota_exceeded",
                "message": "Cota de armazenamento esgotada",
                "message_en": "Storage quota exhausted",
                "used": used,
                "limit": limit,
                "tier": tier,
            });
            return (StatusCode::PAYMENT_REQUIRED, axum::Json(body)).into_response();
        }

        if let AppError::RateLimited { retry_after_secs } = self {
            let body = json!({
                "error": "rate_limited",
                "message": "Muitas requisições. Tente novamente em instantes.",
                "message_en": "Too many requests. Please try again shortly.",
                "retry_after": retry_after_secs,
            });
            let mut resp = (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
            if let Ok(v) = retry_after_secs.to_string().parse() {
                resp.headers_mut().insert("retry-after", v);
            }
            return resp;
        }

        let (status, error, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg),
            AppError::UnprocessableEntity(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "unprocessable_entity",
                msg,
            ),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg),
            AppError::Gone(msg) => (StatusCode::GONE, "gone", msg),
            AppError::TooManyRequests(msg) => {
                (StatusCode::TOO_MANY_REQUESTS, "too_many_requests", msg)
            }
            AppError::UsageLimitExceeded { .. }
            | AppError::RateLimited { .. }
            | AppError::StorageQuotaExceeded { .. } => unreachable!(),
        };

        let message_pt = translate_error_pt(error);
        let body = json!({
            "error": error,
            "message": message_pt,
            "message_en": message,
        });

        (status, axum::Json(body)).into_response()
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        tracing::error!("Database error: {err}");
        AppError::Internal("Database error".into())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        tracing::error!("Internal error: {err}");
        AppError::Internal("Internal server error".into())
    }
}
