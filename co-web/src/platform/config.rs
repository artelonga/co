use clap::Parser;

#[derive(Parser, Clone, Debug)]
#[command(name = "co-web", about = "Project Board — Web UI")]
pub struct Args {
    /// Server port
    #[arg(short, long, env = "CO_WEB_PORT", default_value_t = 8742)]
    pub port: u16,

    /// Bind host. When unset, resolves from `CO_ENV`: `local` -> `127.0.0.1`
    /// (lock down dev), everything else -> `0.0.0.0` (Fly/prod/network deploy).
    /// Set `CO_WEB_HOST=0.0.0.0` to expose a local-env server on the LAN.
    #[arg(long, env = "CO_WEB_HOST")]
    pub web_host: Option<String>,

    /// Data directory path
    #[arg(short, long, env = "CO_WEB_DATA", default_value = "./data")]
    pub data: String,

    /// Static files directory
    #[arg(short, long, env = "CO_WEB_STATIC", default_value = "co-web/static")]
    pub static_dir: String,

    /// Default experiment variant
    #[arg(long, env = "CO_WEB_DEFAULT_VARIANT", default_value = "a")]
    pub default_variant: String,

    /// Enable experiment framework (variant switcher, feedback, A/B analytics)
    #[arg(long, env = "CO_WEB_EXPERIMENTS", default_value_t = true)]
    pub experiments: bool,

    /// Plugins directory path
    #[arg(long, env = "PLUGINS_DIR", default_value = "plugins")]
    pub plugins_dir: String,

    /// Game database path (encrypted redb). Defaults to platform-specific data dir.
    #[arg(long, env = "GAME_DB_PATH")]
    pub game_db_path: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub port: u16,
    pub data_dir: String,
    pub static_dir: String,
    pub default_variant: String,
    pub experiments: bool,
    pub plugins_dir: String,
    pub game_db_path: Option<String>,
    /// Directory for the universo vault files.
    pub universo_dir: String,
    /// GitHub usernames allowed as gestão admins.
    pub gestao_github_admins: Vec<String>,
    /// Optional universe key to scope this server instance to a single universe.
    pub universe_key: Option<String>,
    /// Deployment environment: "prod" (default) or "uat".
    /// Set via the `CO_ENV` environment variable.
    pub co_env: String,
    /// CO-118: WAE Worker proxy URL (e.g. https://wae.co.artelonga.com.br/api/internal/wae).
    /// When absent, WAE emission is a no-op.
    pub wae_endpoint: Option<String>,
    /// CO-118: Bearer token sent to the WAE Worker proxy.
    pub wae_api_key: Option<String>,
    /// CO-166: Domain used for session cookies (e.g. `.artelonga.com.br`).
    /// When absent, no Domain attribute is added (localhost-safe default).
    /// Set via `CO_COOKIE_DOMAIN` env var.
    pub cookie_domain: Option<String>,
    /// CO-208: when true AND `co_env == "test"`, the token-bucket rate-limit
    /// middleware passes every request through unconditionally. Set via
    /// `CO_BYPASS_RATE_LIMIT=1`. Has no effect outside `CO_ENV=test`.
    pub bypass_rate_limit: bool,
}

/// CO-512: resolve the TCP bind host for the web server.
///
/// Rules (in order):
/// 1. an explicit, non-empty `CO_WEB_HOST` always wins;
/// 2. otherwise `co_env == "local"` binds `127.0.0.1` (lock down local dev so a
///    dev server is never reachable from the LAN);
/// 3. otherwise bind `0.0.0.0` (Fly/prod and any network deploy keep working —
///    this preserves the previous unconditional `0.0.0.0` behaviour).
pub fn resolve_bind_host(explicit: Option<&str>, co_env: &str) -> String {
    match explicit {
        Some(h) if !h.is_empty() => h.to_string(),
        _ if co_env == "local" => "127.0.0.1".to_string(),
        _ => "0.0.0.0".to_string(),
    }
}

impl WebConfig {
    /// Returns true when running in UAT mode (`CO_ENV=uat`).
    pub fn is_uat(&self) -> bool {
        self.co_env == "uat"
    }

    /// Returns true when running in staging mode (`CO_ENV=staging`).
    pub fn is_staging(&self) -> bool {
        self.co_env == "staging"
    }

    /// Returns true when UAT-login (`POST /api/v1/auth/uat-login`) should be
    /// reachable. CO-309: returns true for every non-prod environment
    /// (`uat`, `test`, `dev`, `local`, or unset) — same set as
    /// `is_local_or_test()`. Production sets `CO_ENV=prod` explicitly and
    /// is the only deny case.
    pub fn allows_uat_login(&self) -> bool {
        self.is_local_or_test()
    }

    /// Returns true in every non-production environment.
    ///
    /// When true, magic-code login responses include the generated code inline
    /// (CO-303: `dev_code` field) so developers can complete login without
    /// email delivery. Production sets `CO_ENV=prod` explicitly.
    ///
    /// CO-474 (F2): an **empty** `CO_ENV` fails **closed** (treated as prod).
    /// Previously `""` matched here, so a deploy that left `CO_ENV` unset would
    /// leak the inline `dev_code` login code. The production constructor
    /// (`WebConfig::from`) already defaults `CO_ENV` to `"prod"`, so this only
    /// tightens the case where `CO_ENV` is explicitly set to an empty string.
    pub fn is_local_or_test(&self) -> bool {
        matches!(self.co_env.as_str(), "test" | "uat" | "dev" | "local")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_env(env: &str) -> WebConfig {
        WebConfig {
            port: 3000,
            data_dir: String::new(),
            static_dir: String::new(),
            default_variant: "a".into(),
            experiments: false,
            plugins_dir: String::new(),
            game_db_path: None,
            universo_dir: String::new(),
            gestao_github_admins: vec![],
            universe_key: None,
            co_env: env.to_string(),
            wae_endpoint: None,
            wae_api_key: None,
            cookie_domain: None,
            bypass_rate_limit: false,
        }
    }

    #[test]
    fn is_local_or_test_non_prod_envs() {
        for env in &["test", "uat", "dev", "local"] {
            assert!(
                config_with_env(env).is_local_or_test(),
                "expected is_local_or_test() == true for CO_ENV={env:?}"
            );
        }
    }

    #[test]
    fn is_local_or_test_prod_returns_false() {
        for env in &["prod", "production", "staging"] {
            assert!(
                !config_with_env(env).is_local_or_test(),
                "expected is_local_or_test() == false for CO_ENV={env:?}"
            );
        }
    }

    /// CO-474 (F2): empty `CO_ENV` must fail CLOSED (treated as prod), so a
    /// deploy that leaves `CO_ENV` unset never leaks the inline `dev_code`.
    #[test]
    fn is_local_or_test_empty_env_fails_closed() {
        assert!(
            !config_with_env("").is_local_or_test(),
            "empty CO_ENV must NOT be treated as local/test (fail closed → prod-safe)"
        );
    }

    /// CO-512: an explicit `CO_WEB_HOST` always wins, regardless of `CO_ENV`.
    #[test]
    fn resolve_bind_host_explicit_wins() {
        for env in &["local", "prod", "uat", "test", "dev", "", "production"] {
            assert_eq!(
                resolve_bind_host(Some("192.168.1.50"), env),
                "192.168.1.50",
                "explicit CO_WEB_HOST must win for CO_ENV={env:?}"
            );
        }
        // Explicit 0.0.0.0 exposes a local-env server on the LAN (opt-in).
        assert_eq!(resolve_bind_host(Some("0.0.0.0"), "local"), "0.0.0.0");
    }

    /// CO-512: `CO_ENV=local` with no explicit host locks down to loopback.
    #[test]
    fn resolve_bind_host_local_is_loopback() {
        assert_eq!(resolve_bind_host(None, "local"), "127.0.0.1");
        // An empty explicit host is treated as unset (falls through to env rules).
        assert_eq!(resolve_bind_host(Some(""), "local"), "127.0.0.1");
    }

    /// CO-512: prod/unset/uat/test/etc. keep the previous `0.0.0.0` behaviour
    /// so Fly/prod and any network deploy keep working.
    #[test]
    fn resolve_bind_host_non_local_is_all_interfaces() {
        for env in &["prod", "", "uat", "test", "dev", "staging", "production"] {
            assert_eq!(
                resolve_bind_host(None, env),
                "0.0.0.0",
                "non-local CO_ENV must bind 0.0.0.0 for CO_ENV={env:?}"
            );
        }
    }

    #[test]
    fn is_staging_returns_true_only_for_staging() {
        assert!(config_with_env("staging").is_staging());
        for env in &["prod", "uat", "test", "dev", "local", ""] {
            assert!(
                !config_with_env(env).is_staging(),
                "expected is_staging() == false for CO_ENV={env:?}"
            );
        }
    }
}

impl From<Args> for WebConfig {
    fn from(args: Args) -> Self {
        // CO-434: read the remaining env-backed fields through a `SecretsProvider`
        // (the production `EnvSecretsProvider`) at CLI-parse/boot time, instead of
        // calling `std::env::var` directly. Keeps the literal env read confined to
        // `EnvSecretsProvider`.
        use crate::infra::secrets::SecretsProvider;
        let secrets = crate::infra::secrets::EnvSecretsProvider;
        let universo_dir = format!("{}/universes", args.data);
        Self {
            port: args.port,
            data_dir: args.data,
            static_dir: args.static_dir,
            default_variant: args.default_variant,
            experiments: args.experiments,
            plugins_dir: args.plugins_dir,
            game_db_path: args.game_db_path,
            universo_dir,
            gestao_github_admins: secrets
                .get_or("GESTAO_GITHUB_ADMINS", "")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.trim().to_string())
                .collect(),
            universe_key: secrets.get("UNIVERSE_KEY"),
            co_env: secrets.get_or("CO_ENV", "prod"),
            wae_endpoint: secrets.get("WAE_ENDPOINT"),
            wae_api_key: secrets.get("WAE_API_KEY"),
            cookie_domain: secrets.get("CO_COOKIE_DOMAIN"),
            bypass_rate_limit: secrets.get_bool("CO_BYPASS_RATE_LIMIT", false),
        }
    }
}
