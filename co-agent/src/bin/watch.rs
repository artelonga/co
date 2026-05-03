//! `co-agent-watch` — continuous filesystem watcher that streams SyncDeltas
//! over the CO-151 WebSocket protocol.
//!
//! Wraps `co_agent::watcher::SyncWatcher` in a CLI. Used by the launchd
//! plist `scripts/co-sync-v2.plist`.
//!
//! ## Usage
//!
//! ```bash
//! co-agent-watch \
//!     --universe co-dev \
//!     --server-url wss://co-artelonga.fly.dev/api/v1/sync/ws \
//!     --token "$JWT" \
//!     --watch /Users/artelonga/projects/co
//! ```
//!
//! Auto-reconnects on disconnect (5s backoff). Reads token from `--token` or
//! the `CO_SYNC_TOKEN` env var.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use co_agent::watcher::{SyncWatcher, WatcherConfig};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "co-agent-watch", about = "CO-151 sync watcher (protobuf+zstd+WS)")]
struct Cli {
    /// Universe key the watcher belongs to (e.g. `co`, `quilomboaraucaria`).
    #[arg(long, env = "CO_UNIVERSE_KEY")]
    universe: String,

    /// WebSocket endpoint (e.g. `wss://co-artelonga.fly.dev/api/v1/sync/ws`).
    #[arg(
        long,
        env = "CO_SYNC_WS_URL",
        default_value = "wss://co-artelonga.fly.dev/api/v1/sync/ws"
    )]
    server_url: String,

    /// JWT bearer token for auth. Can be set via `CO_SYNC_TOKEN` env var.
    #[arg(long, env = "CO_SYNC_TOKEN")]
    token: String,

    /// Local directory to watch (absolute path). Pass multiple times for
    /// multi-root watching.
    #[arg(long = "watch", required = true)]
    watch_dirs: Vec<PathBuf>,

    /// Resume token from prior session (0 = fresh start).
    #[arg(long, env = "CO_SYNC_RESUME", default_value_t = 0)]
    resume_token: u64,

    /// Backoff seconds before reconnect attempts.
    #[arg(long, default_value_t = 5)]
    reconnect_backoff_secs: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("co_agent=info,co_agent_watch=info")))
        .init();

    let cli = Cli::parse();

    info!(
        universe = %cli.universe,
        server = %cli.server_url,
        watch_count = cli.watch_dirs.len(),
        "co-agent-watch starting"
    );

    let backoff = Duration::from_secs(cli.reconnect_backoff_secs);
    let mut resume_token = cli.resume_token;

    loop {
        let cfg = WatcherConfig {
            watch_dirs: cli.watch_dirs.clone(),
            universe_key: cli.universe.clone(),
            server_url: cli.server_url.clone(),
            auth_token: cli.token.clone(),
            resume_token,
        };
        let watcher = SyncWatcher::new(cfg);

        match watcher.run().await.context("watcher run") {
            Ok(()) => {
                info!("watcher exited cleanly; reconnecting after backoff");
            }
            Err(e) => {
                warn!("watcher error: {e:#}; reconnecting after {backoff:?}");
            }
        }
        tokio::time::sleep(backoff).await;
        // resume_token isn't surfaced from the watcher today (no public getter);
        // pass 0 on reconnect — server falls back to a snapshot if needed.
        resume_token = 0;
    }
}
