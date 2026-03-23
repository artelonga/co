use clap::Parser;

#[derive(Parser, Clone, Debug)]
#[command(name = "co-web", about = "Project Board — Web UI")]
pub struct Args {
    /// Server port
    #[arg(short, long, env = "CO_WEB_PORT", default_value_t = 3000)]
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
}

#[derive(Clone, Debug)]
pub struct WebConfig {
    pub port: u16,
    pub data_dir: String,
    pub static_dir: String,
    pub default_variant: String,
    pub experiments: bool,
}

impl From<Args> for WebConfig {
    fn from(args: Args) -> Self {
        Self {
            port: args.port,
            data_dir: args.data,
            static_dir: args.static_dir,
            default_variant: args.default_variant,
            experiments: args.experiments,
        }
    }
}
