//! `co-sync` — sync local folders with CO universes.
//!
//! ## Setup (one time)
//!
//! 1. Visit <https://co.artelonga.com.br/co/settings/sync>
//! 2. Copy the command shown there
//! 3. Paste and run it
//!
//! ## Usage
//!
//! ```bash
//! # Sync one folder (universe slug detected from _universe.yaml or directory name)
//! co-sync <token> ~/projects/mbya
//!
//! # Sync multiple folders
//! co-sync <token> ~/projects/mbya ~/projects/artelonga ~/projects/topologia
//!
//! # After first run the token is cached — just run without args to resume
//! co-sync
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use co_agent::sync_config::{SyncConfig, UniverseMapping};
use co_agent::watcher::{SyncWatcher, WatcherConfig};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const DEFAULT_SERVER: &str = "https://co.artelonga.com.br";
const CONFIG_PATH_ENV: &str = "CO_SYNC_CONFIG";

#[derive(Parser)]
#[command(
    name = "co-sync",
    about = "Sync local folders with CO — run `co-sync <token> [dirs...]`",
    long_about = "Sync local markdown folders with CO universes.\n\n\
                  Get your token at: https://co.artelonga.com.br/co/settings/sync\n\n\
                  EXAMPLES:\n  \
                  co-sync tok_xxx ~/projects/mbya\n  \
                  co-sync tok_xxx ~/projects/mbya ~/projects/artelonga\n  \
                  co-sync          # resume with cached token"
)]
struct Cli {
    /// API token (get it at /co/settings/sync). Omit to use cached token.
    #[arg(value_name = "TOKEN")]
    token: Option<String>,

    /// Directories to watch. Each must contain a _universe.yaml or the folder
    /// name is used as the universe slug.
    #[arg(value_name = "DIR")]
    dirs: Vec<PathBuf>,

    /// CO server URL (default: https://co.artelonga.com.br)
    #[arg(long, env = "CO_SERVER", default_value = DEFAULT_SERVER)]
    server: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("co_sync=info,co_agent=info")),
        )
        .without_time()
        .init();

    let cli = Cli::parse();
    let config_path = config_path();

    // --- Resolve token ---
    let token = match cli.token {
        Some(t) => {
            // New token provided — save it and the dirs.
            save_config(&config_path, &t, &cli.dirs, &cli.server)?;
            t
        }
        None => {
            // No token — try to load from cache.
            match SyncConfig::load(&config_path) {
                Ok(cfg) => {
                    // If dirs were given on the command line, add them.
                    if !cli.dirs.is_empty() {
                        save_config(&config_path, &cfg.token, &cli.dirs, &cli.server)?;
                    }
                    cfg.token
                }
                Err(_) => {
                    eprintln!("No token found. Get yours at:");
                    eprintln!("  {DEFAULT_SERVER}/co/settings/sync");
                    eprintln!();
                    eprintln!("Then run:");
                    eprintln!("  co-sync <token> [dir...]");
                    std::process::exit(1);
                }
            }
        }
    };

    // --- Resolve directories to watch ---
    let config = SyncConfig::load(&config_path)?;
    if config.universes.is_empty() {
        // No dirs configured — try current directory.
        let cwd = std::env::current_dir()?;
        let slug = detect_slug(&cwd);
        eprintln!("No directories configured. Watching current directory as '{slug}'.");
        eprintln!("To watch specific dirs: co-sync <token> [dir...]");
        eprintln!();
        run_watchers(
            vec![UniverseMapping {
                slug,
                local: cwd.to_string_lossy().into_owned(),
            }],
            &token,
            &config.ws_url,
        )
        .await
    } else {
        run_watchers(config.universes, &token, &config.ws_url).await
    }
}

// ---------------------------------------------------------------------------
// Core sync loop
// ---------------------------------------------------------------------------

async fn run_watchers(universes: Vec<UniverseMapping>, token: &str, ws_url: &str) -> Result<()> {
    if universes.is_empty() {
        bail!("Nothing to watch.");
    }

    for u in &universes {
        info!("▶ {}: {}", u.slug, u.local);
    }
    info!("Syncing {} universe(s) — Ctrl-C to stop", universes.len());

    let mut handles = Vec::new();
    for universe in universes {
        let token = token.to_string();
        let ws_url = ws_url.to_string();
        let handle = tokio::spawn(async move {
            let slug = universe.slug.clone();
            loop {
                let cfg = WatcherConfig {
                    watch_dirs: vec![PathBuf::from(&universe.local)],
                    universe_key: universe.slug.clone(),
                    server_url: ws_url.clone(),
                    auth_token: token.clone(),
                    resume_token: 0,
                };
                match SyncWatcher::new(cfg).run().await {
                    Ok(()) => info!("{slug}: disconnected, reconnecting…"),
                    Err(e) => warn!("{slug}: {e:#}, reconnecting…"),
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        handles.push(handle);
    }

    futures_util::future::join_all(handles).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var(CONFIG_PATH_ENV) {
        return PathBuf::from(p);
    }
    SyncConfig::default_path()
}

/// Detect the universe slug from `_universe.yaml` (the `name:` field slugified)
/// or fall back to the directory name.
fn detect_slug(dir: &Path) -> String {
    let manifest_path = dir.join("_universe.yaml");
    if let Ok(raw) = std::fs::read_to_string(&manifest_path) {
        // Quick parse: look for `name: ...` at the start of a line.
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("name:") {
                let name = rest.trim().trim_matches('"').trim_matches('\'');
                if !name.is_empty() {
                    // Slugify: lowercase, spaces → hyphens, drop non-alphanumeric.
                    return name
                        .to_lowercase()
                        .chars()
                        .map(|c| if c.is_alphanumeric() { c } else { '-' })
                        .collect::<String>()
                        .split('-')
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join("-");
                }
            }
        }
    }
    // Fallback: directory name.
    dir.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_lowercase()
}

/// Save/update config with the given token and directories.
fn save_config(config_path: &Path, token: &str, dirs: &[PathBuf], server: &str) -> Result<()> {
    // Load existing config or start fresh.
    let mut config = SyncConfig::load(config_path).unwrap_or(SyncConfig {
        server: server.to_string(),
        ws_url: server
            .replace("https://", "wss://")
            .replace("http://", "ws://")
            + "/api/v1/sync/ws",
        token: token.to_string(),
        universes: vec![],
    });

    config.token = token.to_string();

    // Add any new directories.
    for dir in dirs {
        let abs = dir
            .canonicalize()
            .with_context(|| format!("cannot resolve '{}'", dir.display()))?;
        let slug = detect_slug(&abs);
        if !config.universes.iter().any(|u| u.slug == slug) {
            info!("Adding {} → {}", slug, abs.display());
            config.universes.push(UniverseMapping {
                slug,
                local: abs.to_string_lossy().into_owned(),
            });
        }
    }

    config.save(config_path)?;
    Ok(())
}
