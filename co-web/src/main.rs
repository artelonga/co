use clap::Parser;

use co_web::config::{Args, WebConfig};
use co_web::server::start_server;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let config = WebConfig::from(args);
    start_server(config).await;
}
