//! CO-434: `CoServerConfig` — the single, boot-time home for every tunable that
//! used to be read with a scattered `std::env::var` at the point of use.
//!
//! It is populated **once** at boot from a [`SecretsProvider`] (see
//! [`CoServerConfig::from_secrets`]) and then handed to subsystems by
//! parameter/state — handlers reach it through [`CoreState`](crate::server::CoreState).
//! Tests inject a `StaticSecretsProvider` to flip behaviour (e.g. the EDA
//! backend) without mutating the global process environment.
//!
//! Classification of the audited reads (CO-434, 2026-06-12):
//! - **Secrets** (credentials/tokens) stay behind `SecretsProvider::get` and are
//!   *not* copied into this struct (so it can derive `Debug` without leaking
//!   them): `JWT_SECRET`, `CO_JWT_PRIVATE_KEY`, `R2_*`, `RESEND_API_KEY`,
//!   `OPENAI_API_KEY`, `CO_SECURITY_API_KEY`, `CO_KB_TOKEN`, `CO_ROLLUP_TOKEN`,
//!   `CO_FLY_API_TOKEN`, `CO_GIT_TOKEN`, `GOOGLE_CLIENT_*`, `GITHUB_OAUTH_*`,
//!   `CO_GITHUB_WEBHOOK_SECRET`, `CO_ASSETS_MASTER_KEY`, `VAPID_PRIVATE_KEY`,
//!   `EVOLUTION_API_KEY`, `CO_SMTP_USER/PASS`, `CO_SEED_ADMIN_PASSWORD_HASH`,
//!   `CO_BRIDGE_OUTBOUND_TOKENS_JSON`, `WAE_API_KEY`.
//! - **Config** (non-sensitive tunables) live here, grouped by subsystem.

use crate::infra::secrets::{SecretsProvider, SecretsProviderExt};

/// Non-secret server configuration, read once at boot from a [`SecretsProvider`].
#[derive(Clone, Debug)]
pub struct CoServerConfig {
    // --- EDA / event bus -------------------------------------------------
    /// `CO_EDA_BACKEND` — event-bus backend (`tokio`). Default: `tokio`.
    pub eda_backend: String,
    /// `CO_DEPLOYMENT_ID` ‖ `FLY_APP_NAME` ‖ `co-local` — federation origin id.
    pub deployment_id: String,
    /// `CO_BRIDGE_TRUSTED_SOURCES` — comma-separated trusted deployment hosts.
    pub bridge_trusted_sources: Vec<String>,
    /// `CO_BRIDGE_HEARTBEAT_S` — bridge heartbeat interval. Default: `30`.
    pub bridge_heartbeat_secs: u64,

    // --- Blob storage ----------------------------------------------------
    /// `CO_BLOB_BACKEND` — `local` (default) or `r2`.
    pub blob_backend: String,
    /// `R2_BUCKET` — bucket name when `blob_backend == "r2"`.
    pub r2_bucket: Option<String>,

    // --- AI / chat -------------------------------------------------------
    /// `CO_CHAT_FALLBACK` — `openai` selects the OpenAI fallback; else Ollama.
    pub chat_fallback: String,
    /// `CO_CHAT_MODEL` — chat model override (provider-specific default applies).
    pub chat_model: Option<String>,
    /// `CO_OLLAMA_URL` — Ollama base URL. Default: `http://localhost:11434`.
    pub ollama_url: String,
    /// `CO_TRANSLATE_BACKEND` — `llm` enables the AiRouter backend; else noop.
    pub translate_backend: String,
    /// `CO_TRANSLATE_PROVIDER` — provider for the LLM translate backend.
    pub translate_provider: String,

    // --- Telemetry -------------------------------------------------------
    /// `CO_TELEMETRY_OTLP_ENDPOINT` — gRPC endpoint; enables OTLP when set.
    pub telemetry_otlp_endpoint: Option<String>,
    /// `CO_TELEMETRY_SERVICE_NAME` — service name. Default: `co-web`.
    pub telemetry_service_name: String,
    /// `CO_TELEMETRY_SAMPLING_RATIO` — trace sampling ratio. Default: `1.0`.
    pub telemetry_sampling_ratio: f64,

    // --- Alerting --------------------------------------------------------
    /// `CO_ALERT_FROM` — degradation-alert From header.
    pub alert_from: String,
    /// `CO_ALERT_TO` — degradation-alert recipient.
    pub alert_to: String,
    /// `CO_ALERT_DEBOUNCE_HOURS` — alert debounce window. Default: `2`.
    pub alert_debounce_hours: u64,

    // --- Security audit --------------------------------------------------
    /// `CO_SECURITY_BACKEND` — `local-grep` (default), `claude`, or `disabled`.
    pub security_backend: String,
    /// `CO_SECURITY_MAX_SCANS_PER_DAY` — daily scan cap. Default: `50`.
    pub security_max_scans_per_day: i64,

    // --- Backup / sync ---------------------------------------------------
    /// `CO_BACKUP_BACKEND` — `local` (default) or `disabled`.
    pub backup_backend: String,
    /// `CO_BACKUP_DIR` — backup directory. Default: `/data/backups/`.
    pub backup_dir: String,
    /// `CO_BACKUP_INTERVAL_HOURS` — backup worker interval. Default: `24`.
    pub backup_interval_hours: u64,
    /// `CO_BACKUP_BOOT_DELAY_SECS` — backup worker boot delay. Default: `600`.
    pub backup_boot_delay_secs: u64,
    /// `CO_REMOTE_SYNC_INTERVAL_SECS` — remote sister-repo sync. Default: `900`.
    pub remote_sync_interval_secs: u64,
    /// `CO_MAINTENANCE_RECLAIM_EVENT_LOG` — when `1`/`true`, run a one-time
    /// boot-time reclaim of the `bridge.*` transport bloat in `event_log`
    /// (batched delete + VACUUM). Idempotent; safe to leave on (no-op once clean).
    pub maintenance_reclaim_event_log: bool,

    // --- Seeding / admin -------------------------------------------------
    /// `CO_SEED_ADMIN_EMAIL` — admin email gating dev/lead routes.
    pub seed_admin_email: Option<String>,
    /// `CO_DEV_OWNER` — subject id authorised for the co-dev board.
    pub dev_owner: Option<String>,
    /// `LEADS_NOTIFY_TO` — lead-notification recipient.
    pub leads_notify_to: String,
    /// `CO_LOCAL_REPOS_DIR` — local sister-repo root (dev seeding override).
    pub local_repos_dir: Option<String>,
    /// `CO_SEED_CO_DIR` — explicit CO seed dir (UAT boot).
    pub seed_co_dir: Option<String>,

    // --- Paths / models --------------------------------------------------
    /// `CO_MODELS_DIR` — embedding model cache dir override.
    pub models_dir: Option<String>,
    /// `GEOIP_DB_PATH` — GeoLite2 DB path. Default: `/data/GeoLite2-City.mmdb`.
    pub geoip_db_path: String,

    // --- Networking / hosts ----------------------------------------------
    /// `CO_TRUSTED_IPS` — comma-separated trusted CIDRs for rate-limit bypass.
    pub trusted_ips: String,
    /// `CO_STATIC_SITES` — comma-separated slugs served by static apps.
    pub static_sites: Option<String>,
    /// `CO_PUBLIC_URL` — public issuer URL (OIDC).
    pub public_url: Option<String>,
    /// `CO_BASE_URL` — base URL for invitation links.
    pub base_url: String,
    /// `CANONICAL_HOST` — canonical-host redirect target.
    pub canonical_host: Option<String>,
    /// `ALLOWED_ORIGINS` — comma-separated CORS origins.
    pub allowed_origins: String,
    /// `CO_FEEDBACK_FORWARD_URL` — feedback webhook forward target.
    pub feedback_forward_url: Option<String>,

    // --- Email / notifications -------------------------------------------
    /// `NOTIF_FROM_EMAIL` — security-notification From.
    pub notif_from_email: String,
    /// `RESEND_FROM` — Resend channel From.
    pub resend_from: String,
    /// `VAPID_SUBJECT` — Web Push VAPID subject.
    pub vapid_subject: String,
    /// `EVOLUTION_API_URL` — Evolution (WhatsApp) API URL.
    pub evolution_api_url: String,
    /// `EVOLUTION_INSTANCE` — Evolution instance name.
    pub evolution_instance: String,
    /// `CO_SMTP_HOST` — SMTP host (mailer disabled when absent).
    pub smtp_host: Option<String>,
    /// `CO_SMTP_FROM` — SMTP From header.
    pub smtp_from: Option<String>,
    /// `CO_SMTP_PORT` — SMTP port. Default: `587`.
    pub smtp_port: u16,

    // --- Misc toggles ----------------------------------------------------
    /// `CO_DESKTOP_NOTIFY` — `off` disables; default on for macOS.
    pub desktop_notify_enabled: bool,
    /// `CO_EMBEDDING_BOOT_SCAN` — opt-in boot embedding backfill. Default: off.
    pub embedding_boot_scan: bool,
}

impl CoServerConfig {
    /// Populate the config once, reading every tunable through `secrets`.
    pub fn from_secrets(secrets: &dyn SecretsProvider) -> Self {
        let desktop_notify_enabled = match secrets.get("CO_DESKTOP_NOTIFY") {
            Some(v) => v.to_lowercase() != "off",
            None => cfg!(target_os = "macos"),
        };
        Self {
            eda_backend: secrets.get_or("CO_EDA_BACKEND", "tokio"),
            deployment_id: secrets
                .get("CO_DEPLOYMENT_ID")
                .or_else(|| secrets.get("FLY_APP_NAME"))
                .unwrap_or_else(|| "co-local".into()),
            bridge_trusted_sources: split_csv(&secrets.get_or("CO_BRIDGE_TRUSTED_SOURCES", "")),
            bridge_heartbeat_secs: secrets.get_parsed("CO_BRIDGE_HEARTBEAT_S", 30),

            blob_backend: secrets.get_or("CO_BLOB_BACKEND", "local"),
            r2_bucket: secrets.get("R2_BUCKET"),

            chat_fallback: secrets.get_or("CO_CHAT_FALLBACK", ""),
            chat_model: secrets.get("CO_CHAT_MODEL"),
            ollama_url: secrets.get_or("CO_OLLAMA_URL", "http://localhost:11434"),
            translate_backend: secrets.get_or("CO_TRANSLATE_BACKEND", ""),
            translate_provider: secrets.get_or("CO_TRANSLATE_PROVIDER", "ollama"),

            telemetry_otlp_endpoint: secrets.get_nonempty("CO_TELEMETRY_OTLP_ENDPOINT"),
            telemetry_service_name: secrets.get_or("CO_TELEMETRY_SERVICE_NAME", "co-web"),
            telemetry_sampling_ratio: secrets
                .get_parsed::<f64>("CO_TELEMETRY_SAMPLING_RATIO", 1.0)
                .clamp(0.0, 1.0),

            // Sender must be on a Resend-verified domain. `artelonga.com.br` is NOT
            // verified (alerts silently failed / spam-foldered); `seguranca.artelonga.com.br`
            // is the verified domain used for password mail (`senhas@…`).
            alert_from: secrets.get_or(
                "CO_ALERT_FROM",
                "CO Alertas <alertas@seguranca.artelonga.com.br>",
            ),
            alert_to: secrets.get_or("CO_ALERT_TO", "yuri@artelonga.com.br"),
            alert_debounce_hours: secrets.get_parsed("CO_ALERT_DEBOUNCE_HOURS", 2),

            security_backend: secrets.get_or("CO_SECURITY_BACKEND", "local-grep"),
            security_max_scans_per_day: secrets.get_parsed("CO_SECURITY_MAX_SCANS_PER_DAY", 50),

            backup_backend: secrets.get_or("CO_BACKUP_BACKEND", "local"),
            backup_dir: secrets.get_or("CO_BACKUP_DIR", "/data/backups/"),
            backup_interval_hours: secrets.get_parsed("CO_BACKUP_INTERVAL_HOURS", 24),
            backup_boot_delay_secs: secrets.get_parsed("CO_BACKUP_BOOT_DELAY_SECS", 600),
            remote_sync_interval_secs: secrets.get_parsed("CO_REMOTE_SYNC_INTERVAL_SECS", 900),
            maintenance_reclaim_event_log: secrets
                .get_bool("CO_MAINTENANCE_RECLAIM_EVENT_LOG", false),

            seed_admin_email: secrets.get_nonempty("CO_SEED_ADMIN_EMAIL"),
            dev_owner: secrets.get_nonempty("CO_DEV_OWNER"),
            leads_notify_to: secrets.get_or("LEADS_NOTIFY_TO", "rede@artelonga.com.br"),
            local_repos_dir: secrets.get("CO_LOCAL_REPOS_DIR"),
            seed_co_dir: secrets.get("CO_SEED_CO_DIR"),

            models_dir: secrets.get("CO_MODELS_DIR"),
            geoip_db_path: secrets.get_or("GEOIP_DB_PATH", "/data/GeoLite2-City.mmdb"),

            trusted_ips: secrets.get_or("CO_TRUSTED_IPS", ""),
            static_sites: secrets.get("CO_STATIC_SITES"),
            public_url: secrets.get_nonempty("CO_PUBLIC_URL"),
            base_url: secrets.get_or("CO_BASE_URL", "https://co.artelonga.com.br"),
            canonical_host: secrets.get_nonempty("CANONICAL_HOST"),
            allowed_origins: secrets.get_or("ALLOWED_ORIGINS", ""),
            feedback_forward_url: secrets.get_nonempty("CO_FEEDBACK_FORWARD_URL"),

            notif_from_email: secrets.get_or(
                "NOTIF_FROM_EMAIL",
                "notificacoes@seguranca.artelonga.com.br",
            ),
            // Default to the verified seguranca.artelonga.com.br domain, which is
            // Resend-verified so this channel's mail can send (overridable via RESEND_FROM).
            resend_from: secrets.get_or("RESEND_FROM", "CO <noreply@seguranca.artelonga.com.br>"),
            vapid_subject: secrets.get_or("VAPID_SUBJECT", "mailto:noreply@co.artelonga.com.br"),
            evolution_api_url: secrets.get_or("EVOLUTION_API_URL", "https://api.evolution-api.com"),
            evolution_instance: secrets.get_or("EVOLUTION_INSTANCE", "default"),
            smtp_host: secrets.get("CO_SMTP_HOST"),
            smtp_from: secrets.get("CO_SMTP_FROM"),
            smtp_port: secrets.get_parsed("CO_SMTP_PORT", 587),

            desktop_notify_enabled,
            embedding_boot_scan: secrets.get_bool("CO_EMBEDDING_BOOT_SCAN", false),
        }
    }
}

impl Default for CoServerConfig {
    /// Defaults equal to a boot with no env vars set (an empty provider).
    fn default() -> Self {
        Self::from_secrets(&crate::infra::secrets::StaticEmpty)
    }
}

/// Split a comma-separated list, trimming whitespace and dropping empties.
fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::secrets::StaticSecretsProvider;

    #[test]
    fn defaults_match_no_env_boot() {
        let cfg = CoServerConfig::default();
        assert_eq!(cfg.eda_backend, "tokio");
        assert_eq!(cfg.blob_backend, "local");
        assert_eq!(cfg.deployment_id, "co-local");
        assert_eq!(cfg.telemetry_sampling_ratio, 1.0);
        assert_eq!(cfg.security_max_scans_per_day, 50);
        assert_eq!(cfg.base_url, "https://co.artelonga.com.br");
    }

    #[test]
    fn provider_swaps_behaviour_without_global_env() {
        // CO-434 acceptance: a StaticSecretsProvider changes config (e.g. the
        // EDA backend) with no mutation of the global process environment.
        let provider = StaticSecretsProvider::new([
            ("CO_EDA_BACKEND", "redis"),
            ("CO_BLOB_BACKEND", "r2"),
            ("CO_TELEMETRY_SAMPLING_RATIO", "0.25"),
        ]);
        let cfg = CoServerConfig::from_secrets(&*provider);
        assert_eq!(cfg.eda_backend, "redis");
        assert_eq!(cfg.blob_backend, "r2");
        assert_eq!(cfg.telemetry_sampling_ratio, 0.25);
        // The real process env is untouched.
        assert!(std::env::var("CO_EDA_BACKEND").is_err());
    }
}
