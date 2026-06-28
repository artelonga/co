use clap::Parser;

use co_web::config::{Args, WebConfig, resolve_bind_host};
use co_web::server::start_server_on;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    // CO-512: capture the explicit CO_WEB_HOST before `args` is consumed; the
    // resolver also needs the resolved `CO_ENV`, which lives on `WebConfig`.
    let web_host = args.web_host.clone();
    let config = WebConfig::from(args);
    // CO-512: resolve the bind host (explicit CO_WEB_HOST > CO_ENV default).
    let bind_host = resolve_bind_host(web_host.as_deref(), &config.co_env);
    start_server_on(config, &bind_host).await;
}
