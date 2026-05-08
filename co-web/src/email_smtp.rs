//! CO-165: SMTP email delivery for recovery codes.
//!
//! When all of `CO_SMTP_HOST`, `CO_SMTP_USER`, `CO_SMTP_PASS`, `CO_SMTP_FROM`
//! are set, recovery codes are delivered via SMTP (lettre, rustls-TLS, port
//! 587 STARTTLS by default; override with `CO_SMTP_PORT`). When any are
//! missing the function returns `Ok(false)` so the caller can fall back to
//! logging the code (the existing dev pattern).
//!
//! Send is best-effort: a delivery failure logs at WARN and returns `Err`,
//! but the caller treats this the same as the no-SMTP branch — log the code
//! to stderr so a developer / operator can still recover. We *never* leak the
//! code to the client.

use std::env;

use lettre::{
    Message, Tokio1Executor,
    message::header::ContentType,
    transport::smtp::{AsyncSmtpTransport, authentication::Credentials},
    AsyncTransport,
};

/// SMTP configuration assembled from env vars. `None` if any required field
/// is missing — caller falls back to logging the code.
struct SmtpConfig {
    host: String,
    port: u16,
    username: String,
    password: String,
    from: String,
}

impl SmtpConfig {
    fn from_env() -> Option<Self> {
        let host = env::var("CO_SMTP_HOST").ok()?;
        let username = env::var("CO_SMTP_USER").ok()?;
        let password = env::var("CO_SMTP_PASS").ok()?;
        let from = env::var("CO_SMTP_FROM").ok()?;
        let port = env::var("CO_SMTP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(587);
        Some(Self {
            host,
            port,
            username,
            password,
            from,
        })
    }
}

/// Send a recovery verification code by email.
///
/// Returns:
/// - `Ok(true)`  — SMTP configured and message accepted by the relay.
/// - `Ok(false)` — SMTP not configured (caller should log the code).
/// - `Err(_)`    — SMTP configured but delivery failed.
pub async fn send_recovery_code(to: &str, code: &str) -> anyhow::Result<bool> {
    let cfg = match SmtpConfig::from_env() {
        Some(c) => c,
        None => return Ok(false),
    };

    let subject = "Seu código de recuperação CO";
    let body = format!(
        "Olá,\n\n\
         Use este código para recuperar sua conta CO:\n\n\
         \t{code}\n\n\
         O código expira em 10 minutos. Se você não solicitou esta recuperação,\n\
         pode ignorar este email — sua conta está segura.\n\n\
         — CO\n"
    );

    let email = Message::builder()
        .from(cfg.from.parse()?)
        .to(to.parse()?)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body)?;

    let creds = Credentials::new(cfg.username.clone(), cfg.password.clone());
    let mailer: AsyncSmtpTransport<Tokio1Executor> =
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)?
            .port(cfg.port)
            .credentials(creds)
            .build();

    mailer
        .send(email)
        .await
        .map(|_| true)
        .map_err(|e| anyhow::anyhow!("SMTP send failed: {e}"))
}
