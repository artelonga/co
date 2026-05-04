//! `co-sync` — multi-universe sync daemon for CO.
//!
//! # One-time setup
//!
//! ```bash
//! co-sync init --email yuri@artelonga.com.br
//! ```
//!
//! Authenticates once (prompts for password), creates a permanent API token,
//! writes `~/.co/sync.toml`. No password ever needed again.
//!
//! Then add universes to watch:
//!
//! ```bash
//! co-sync add --slug co    --local /Users/yuri/projects/co
//! co-sync add --slug mbya  --local /Users/yuri/projects/mbya
//! co-sync add --slug topologia --local /Users/yuri/projects/topologia
//! ```
//!
//! # Running
//!
//! ```bash
//! co-sync run          # foreground, watches all configured universes
//! co-sync install      # installs ~/Library/LaunchAgents/com.artelonga.co-sync.plist
//! co-sync status       # show configured universes and token info
//! ```

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use co_agent::sync_config::{SyncConfig, UniverseMapping};
use co_agent::watcher::{SyncWatcher, WatcherConfig};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "co-sync", about = "CO multi-universe sync daemon")]
struct Cli {
    /// Config file path (default: ~/.co/sync.toml).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Authenticate once and create a permanent API token.
    Init {
        #[arg(long, default_value = "https://co.artelonga.com.br")]
        server: String,
        #[arg(long)]
        email: String,
    },
    /// Add a universe → local directory mapping.
    Add {
        #[arg(long)]
        slug: String,
        #[arg(long)]
        local: String,
    },
    /// Remove a universe mapping.
    Remove {
        #[arg(long)]
        slug: String,
    },
    /// Print current configuration and token status.
    Status,
    /// Start watching all configured universes (runs until killed).
    Run,
    /// Install a launchd plist so co-sync runs automatically at login (macOS).
    Install,
    /// Unload and remove the launchd plist.
    Uninstall,
}

// ---------------------------------------------------------------------------
// REST types (subset of co-web API)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct PasswordLoginRequest<'a> {
    email: &'a str,
    password: &'a str,
}

#[derive(serde::Serialize)]
struct CreateTokenRequest<'a> {
    name: &'a str,
}

#[derive(serde::Deserialize)]
struct CreateTokenResponse {
    token: String,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("co_sync=info,co_agent=info")),
        )
        .init();

    let cli = Cli::parse();
    let config_path = cli.config.unwrap_or_else(SyncConfig::default_path);

    match cli.command {
        Command::Init { server, email } => cmd_init(&config_path, &server, &email).await,
        Command::Add { slug, local } => cmd_add(&config_path, slug, local),
        Command::Remove { slug } => cmd_remove(&config_path, &slug),
        Command::Status => cmd_status(&config_path),
        Command::Run => cmd_run(&config_path).await,
        Command::Install => cmd_install(&config_path),
        Command::Uninstall => cmd_uninstall(),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

async fn cmd_init(config_path: &std::path::Path, server: &str, email: &str) -> Result<()> {
    let password = rpassword::prompt_password(format!("Password for {email}: "))?;

    println!("Authenticating with {server}…");
    let client = reqwest::Client::new();

    // 1. Password login — JWT is returned in the Set-Cookie: session=<jwt> header.
    let login_resp = client
        .post(format!("{server}/api/v1/auth/password-login"))
        .json(&PasswordLoginRequest {
            email,
            password: &password,
        })
        .send()
        .await
        .context("password login request")?;

    let status = login_resp.status();
    // Extract the JWT from Set-Cookie before consuming the body.
    let jwt_opt = login_resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| {
            s.split(';')
                .next()
                .and_then(|p| p.strip_prefix("session="))
                .map(str::to_string)
        });

    if !status.is_success() {
        let body = login_resp.text().await.unwrap_or_default();
        bail!("Login failed ({status}): {body}");
    }

    let jwt = jwt_opt.ok_or_else(|| {
        anyhow::anyhow!(
            "Login OK but no session cookie in response — \
             check that password-login is enabled for this account"
        )
    })?;

    // 2. Create long-lived API token
    let token_res: CreateTokenResponse = client
        .post(format!("{server}/api/v1/auth/token"))
        .bearer_auth(&jwt)
        .json(&CreateTokenRequest { name: "co-sync" })
        .send()
        .await?
        .json()
        .await
        .context("creating API token")?;

    let ws_url = server
        .replace("https://", "wss://")
        .replace("http://", "ws://");
    let ws_url = format!("{ws_url}/api/v1/sync/ws");

    let config = SyncConfig {
        server: server.to_string(),
        ws_url,
        token: token_res.token,
        universes: vec![],
    };
    config.save(config_path)?;

    println!("✓ API token created and saved to {}", config_path.display());
    println!("  Token never expires — no password needed again.");
    println!();
    println!("Next: add universes to watch:");
    println!("  co-sync add --slug co    --local /path/to/co");
    println!("  co-sync add --slug mbya  --local /path/to/mbya");
    println!("  co-sync run");
    Ok(())
}

fn cmd_add(config_path: &std::path::Path, slug: String, local: String) -> Result<()> {
    let mut config = load_or_empty(config_path)?;
    if config.universes.iter().any(|u| u.slug == slug) {
        bail!("Universe '{slug}' already configured. Remove it first.");
    }
    let abs = std::fs::canonicalize(&local)
        .with_context(|| format!("cannot resolve '{local}'"))?
        .to_string_lossy()
        .into_owned();
    config.universes.push(UniverseMapping {
        slug: slug.clone(),
        local: abs.clone(),
    });
    config.save(config_path)?;
    println!("✓ Added {slug} → {abs}");
    Ok(())
}

fn cmd_remove(config_path: &std::path::Path, slug: &str) -> Result<()> {
    let mut config = load_config(config_path)?;
    let before = config.universes.len();
    config.universes.retain(|u| u.slug != slug);
    if config.universes.len() == before {
        bail!("Universe '{slug}' not found in config.");
    }
    config.save(config_path)?;
    println!("✓ Removed {slug}");
    Ok(())
}

fn cmd_status(config_path: &std::path::Path) -> Result<()> {
    match load_config(config_path) {
        Err(_) => {
            println!("No config found at {}.", config_path.display());
            println!("Run `co-sync init` to get started.");
        }
        Ok(cfg) => {
            println!("Config: {}", config_path.display());
            println!("Server: {}", cfg.server);
            let tok_preview = if cfg.token.len() > 12 {
                format!("{}…{}", &cfg.token[..8], &cfg.token[cfg.token.len() - 4..])
            } else {
                "(empty)".to_string()
            };
            println!("Token:  {tok_preview}");
            println!();
            if cfg.universes.is_empty() {
                println!(
                    "No universes configured. Use `co-sync add --slug <slug> --local <path>`."
                );
            } else {
                println!("Universes ({}):", cfg.universes.len());
                for u in &cfg.universes {
                    println!("  {} → {}", u.slug, u.local);
                }
            }
        }
    }
    Ok(())
}

async fn cmd_run(config_path: &std::path::Path) -> Result<()> {
    let config = load_config(config_path)?;

    if config.universes.is_empty() {
        bail!("No universes configured. Add some with `co-sync add`.");
    }
    if config.token.is_empty() {
        bail!("No token found. Run `co-sync init` first.");
    }

    info!(
        universes = config.universes.len(),
        server = %config.server,
        "co-sync starting"
    );

    let mut handles = Vec::new();
    for universe in config.universes.clone() {
        let token = config.token.clone();
        let ws_url = config.ws_url.clone();
        let handle = tokio::spawn(async move {
            let slug = universe.slug.clone();
            info!(slug, local = %universe.local, "starting watcher");
            loop {
                let cfg = WatcherConfig {
                    watch_dirs: vec![PathBuf::from(&universe.local)],
                    universe_key: universe.slug.clone(),
                    server_url: ws_url.clone(),
                    auth_token: token.clone(),
                    resume_token: 0,
                };
                let watcher = SyncWatcher::new(cfg);
                match watcher.run().await {
                    Ok(()) => info!(slug, "watcher disconnected; reconnecting"),
                    Err(e) => warn!(slug, "watcher error: {e:#}; reconnecting"),
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        handles.push(handle);
    }

    futures_util::future::join_all(handles).await;
    Ok(())
}

fn cmd_install(config_path: &std::path::Path) -> Result<()> {
    #[cfg(not(target_os = "macos"))]
    {
        println!("Install is only supported on macOS (launchd).");
        println!("On Linux, create a systemd user service instead:");
        println!("  ~/.config/systemd/user/co-sync.service");
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let binary = std::env::current_exe()?.to_string_lossy().into_owned();
        let config_str = config_path.to_string_lossy();
        let home = dirs_next::home_dir().context("no home dir")?;
        let plist_dir = home.join("Library/LaunchAgents");
        std::fs::create_dir_all(&plist_dir)?;
        let plist_path = plist_dir.join("com.artelonga.co-sync.plist");

        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!--
  co-sync launchd plist — generated by `co-sync install`.
  Token is stored in ~/.co/sync.toml (chmod 600), not here.
  Re-run `co-sync install` after updating the binary path.
-->
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.artelonga.co-sync</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>run</string>
        <string>--config</string>
        <string>{config_str}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>5</integer>
    <key>StandardOutPath</key>
    <string>{home}/.co/co-sync-stdout.log</string>
    <key>StandardErrorPath</key>
    <string>{home}/.co/co-sync-stderr.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>co_sync=info,co_agent=info</string>
    </dict>
</dict>
</plist>"#,
            binary = binary,
            config_str = config_str,
            home = home.display(),
        );

        std::fs::write(&plist_path, plist.as_bytes())?;
        println!("✓ Plist written to {}", plist_path.display());

        let load = std::process::Command::new("launchctl")
            .args(["load", plist_path.to_str().unwrap()])
            .output()?;
        if load.status.success() {
            println!("✓ Loaded into launchd — co-sync will run at login automatically.");
        } else {
            println!(
                "! launchctl load failed: {}",
                String::from_utf8_lossy(&load.stderr)
            );
            println!("  Try: launchctl load {}", plist_path.display());
        }
        println!();
        println!("Logs:");
        println!("  tail -f {}/.co/co-sync-stdout.log", home.display());
    }
    Ok(())
}

fn cmd_uninstall() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs_next::home_dir().context("no home dir")?;
        let plist_path = home.join("Library/LaunchAgents/com.artelonga.co-sync.plist");
        if plist_path.exists() {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", plist_path.to_str().unwrap()])
                .output();
            std::fs::remove_file(&plist_path)?;
            println!("✓ Unloaded and removed {}", plist_path.display());
        } else {
            println!("No plist found at {}", plist_path.display());
        }
    }
    #[cfg(not(target_os = "macos"))]
    println!("Uninstall is only supported on macOS.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_config(path: &std::path::Path) -> Result<SyncConfig> {
    SyncConfig::load(path).with_context(|| {
        format!(
            "Cannot load config from {}. Run `co-sync init` first.",
            path.display()
        )
    })
}

fn load_or_empty(path: &std::path::Path) -> Result<SyncConfig> {
    if path.exists() {
        load_config(path)
    } else {
        bail!("No config at {}. Run `co-sync init` first.", path.display())
    }
}
