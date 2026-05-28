use clap::Parser;

#[derive(Parser, Clone, Debug)]
#[command(name = "co-web", about = "Project Board — Web UI")]
pub struct Args {
    /// Server port
    #[arg(short, long, env = "CO_WEB_PORT", default_value_t = 8742)]
    pub port: u16,

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
    /// Directory for the universo (quilombo) vault files.
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
    /// CO-166: When false, `POST /api/v1/quilombo/auth/login` returns 410 Gone.
    /// Default: true (legacy login enabled). Set via `CO_QUILOMBO_LEGACY_LOGIN=false`.
    pub quilombo_legacy_login: bool,
    /// CO-208: when true AND `co_env == "test"`, the token-bucket rate-limit
    /// middleware passes every request through unconditionally. Set via
    /// `CO_BYPASS_RATE_LIMIT=1`. Has no effect outside `CO_ENV=test`.
    pub bypass_rate_limit: bool,
}

impl WebConfig {
    /// Returns true when running in UAT mode (`CO_ENV=uat`).
    pub fn is_uat(&self) -> bool {
        self.co_env == "uat"
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
    pub fn is_local_or_test(&self) -> bool {
        matches!(self.co_env.as_str(), "test" | "uat" | "dev" | "local" | "")
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
            quilombo_legacy_login: true,
            bypass_rate_limit: false,
        }
    }

    #[test]
    fn is_local_or_test_non_prod_envs() {
        for env in &["test", "uat", "dev", "local", ""] {
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
}

impl From<Args> for WebConfig {
    fn from(args: Args) -> Self {
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
            gestao_github_admins: std::env::var("GESTAO_GITHUB_ADMINS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.trim().to_string())
                .collect(),
            universe_key: std::env::var("UNIVERSE_KEY").ok(),
            co_env: std::env::var("CO_ENV").unwrap_or_else(|_| "prod".into()),
            wae_endpoint: std::env::var("WAE_ENDPOINT").ok(),
            wae_api_key: std::env::var("WAE_API_KEY").ok(),
            cookie_domain: std::env::var("CO_COOKIE_DOMAIN").ok(),
            quilombo_legacy_login: std::env::var("CO_QUILOMBO_LEGACY_LOGIN")
                .map(|v| v != "false")
                .unwrap_or(true),
            bypass_rate_limit: std::env::var("CO_BYPASS_RATE_LIMIT")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        }
    }
}
