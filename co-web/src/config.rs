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
}

impl WebConfig {
    /// Returns true when running in UAT mode (`CO_ENV=uat`).
    pub fn is_uat(&self) -> bool {
        self.co_env == "uat"
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
        }
    }
}
