//! co-agent — CO telemetry sidecar
//!
//! Reads log lines from stdin, wraps each in a TelemetryEvent, and ships them
//! in compressed, HMAC-signed batches to the CO ingest endpoint.
//!
//! Configuration via environment variables (all required unless defaulted):
//!
//!   CO_INGEST_URL      Ingest endpoint  (default: https://ingest.co.artelonga.com.br/v1/events)
//!   CO_UNIVERSE_ID     Universe ID this sidecar belongs to
//!   CO_HMAC_KEY        HMAC-SHA256 signing key, hex-encoded (32 bytes recommended)
//!   CO_BATCH_SIZE      Events per batch          (default: 200)
//!   CO_FLUSH_INTERVAL  Seconds between flushes   (default: 10)
//!   CO_MAX_RETRIES     POST attempts before drop  (default: 3)

use std::io::{BufRead, BufReader};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tracing::{error, info};

use co::agent::{AgentConfig, CoAgent, TelemetryEvent};
use co_agent::FlySidecarAgent;

#[derive(Parser)]
#[command(name = "co-agent", about = "CO telemetry sidecar")]
struct Args {
    #[arg(
        long,
        env = "CO_INGEST_URL",
        default_value = "https://ingest.co.artelonga.com.br/v1/events"
    )]
    ingest_url: String,

    #[arg(long, env = "CO_UNIVERSE_ID")]
    universe_id: String,

    /// HMAC key as a hex string (e.g. the output of `openssl rand -hex 32`).
    #[arg(long, env = "CO_HMAC_KEY")]
    hmac_key: String,

    #[arg(long, env = "CO_BATCH_SIZE", default_value = "200")]
    batch_size: usize,

    #[arg(long, env = "CO_FLUSH_INTERVAL", default_value = "10")]
    flush_interval: u64,

    #[arg(long, env = "CO_MAX_RETRIES", default_value = "3")]
    max_retries: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let hmac_key =
        hex::decode(&args.hmac_key).context("CO_HMAC_KEY must be a hex-encoded byte string")?;
    if hmac_key.is_empty() {
        bail!("CO_HMAC_KEY must not be empty");
    }

    let config = AgentConfig {
        batch_size: args.batch_size,
        flush_interval_secs: args.flush_interval,
        max_retries: args.max_retries,
        ..Default::default()
    };

    let agent = std::sync::Arc::new(
        FlySidecarAgent::new(args.ingest_url, &args.universe_id, hmac_key, config.clone())
            .context("init agent")?,
    );

    info!(universe_id = %args.universe_id, "co-agent started");

    // Flush interval timer
    let flush_agent = agent.clone();
    let flush_interval = Duration::from_secs(config.flush_interval_secs);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(flush_interval);
        ticker.tick().await; // skip the immediate first tick
        loop {
            ticker.tick().await;
            if let Err(e) = flush_agent.flush().await {
                error!(err = %e, "flush error");
            }
        }
    });

    // Heartbeat timer (every 60 s)
    let hb_agent = agent.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(e) = hb_agent.heartbeat().await {
                error!(err = %e, "heartbeat error");
            }
        }
    });

    // Graceful shutdown: flush on SIGTERM / SIGINT
    let shutdown_agent = agent.clone();
    let shutdown = async move {
        tokio::signal::ctrl_c().await.ok();
        info!("signal received — flushing remaining events");
        if let Err(e) = shutdown_agent.flush().await {
            error!(err = %e, "flush on shutdown");
        }
    };

    // Main loop: read log lines from stdin
    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    let universe_id = args.universe_id.clone();

    let read_loop = async move {
        for line in reader.lines() {
            match line {
                Ok(text) if text.is_empty() => continue,
                Ok(text) => {
                    let event = TelemetryEvent::new(
                        &universe_id,
                        "log",
                        serde_json::json!({ "message": text }),
                    );
                    if let Err(e) = agent.ship(&[event]).await {
                        error!(err = %e, "ship error");
                    }
                }
                Err(e) => {
                    error!(err = %e, "stdin read error");
                    break;
                }
            }
        }
        info!("stdin closed — flushing");
        agent.flush().await.ok();
    };

    tokio::select! {
        _ = read_loop => {}
        _ = shutdown => {}
    }

    Ok(())
}
