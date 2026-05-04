//! `co-sync` — proactive local→CO sync daemon.
//!
//! Dual-channel, git-independent:
//!   • Markdown  → WebSocket SyncDelta (real-time, bidirectional)
//!   • Binary    → REST asset upload (PDF, image, video — on-change)
//!
//! On startup: full REST push of every file in the watched directories,
//! then switches to filesystem-event-driven incremental updates.
//!
//! Setup (one time):
//!   1. Log in at co.artelonga.com.br
//!   2. Visit  co.artelonga.com.br/co/settings/sync
//!   3. Copy the command — paste and run.
//!
//! Usage:
//!   co-sync <token> ~/projects/mbya ~/projects/artelonga
//!   co-sync                         # resume with cached config

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use co_agent::sync_config::{SyncConfig, UniverseMapping};
use co_agent::watcher::{SyncWatcher, WatcherConfig};
use notify::RecursiveMode;
use notify::Watcher as _;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const DEFAULT_SERVER: &str = "https://co.artelonga.com.br";
const CONFIG_PATH_ENV: &str = "CO_SYNC_CONFIG";

// Files to never sync.
const IGNORE_NAMES: &[&str] = &[".DS_Store", ".git", ".svn", "node_modules", "target", ".co"];

#[derive(Parser)]
#[command(
    name = "co-sync",
    about = "Proactive local→CO sync — no git required",
    long_about = "Syncs every file in your local folders to CO universes.\n\n\
                  Get your token at: https://co.artelonga.com.br/co/settings/sync\n\n\
                  EXAMPLES:\n  \
                  co-sync tok_xxx ~/projects/mbya ~/projects/artelonga\n  \
                  co-sync                   # resume"
)]
struct Cli {
    #[arg(value_name = "TOKEN")]
    token: Option<String>,
    #[arg(value_name = "DIR")]
    dirs: Vec<PathBuf>,
    #[arg(long, env = "CO_SERVER", default_value = DEFAULT_SERVER)]
    server: String,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

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

    let (token, server) = match cli.token {
        Some(t) => {
            let s = cli.server.clone();
            save_config(&config_path, &t, &cli.dirs, &s)?;
            (t, s)
        }
        None => match SyncConfig::load(&config_path) {
            Ok(cfg) => {
                if !cli.dirs.is_empty() {
                    save_config(&config_path, &cfg.token, &cli.dirs, &cfg.server)?;
                }
                let s = cfg.server.clone();
                (cfg.token, s)
            }
            Err(_) => {
                eprintln!("No token. Get yours at: {DEFAULT_SERVER}/co/settings/sync");
                eprintln!("Then run: co-sync <token> [dir...]");
                std::process::exit(1);
            }
        },
    };

    let config = SyncConfig::load(&config_path)?;
    let universes = if config.universes.is_empty() {
        let cwd = std::env::current_dir()?;
        let slug = detect_slug(&cwd);
        eprintln!(
            "Watching current directory as '{slug}'. Use co-sync <token> [dir...] to configure."
        );
        vec![UniverseMapping {
            slug,
            local: cwd.to_string_lossy().into_owned(),
        }]
    } else {
        config.universes
    };

    let http = Arc::new(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?,
    );

    // Run all universes concurrently.
    let mut handles = Vec::new();
    for universe in universes {
        let token = token.clone();
        let server = server.clone();
        let ws_url = config.ws_url.clone();
        let http = Arc::clone(&http);
        handles.push(tokio::spawn(async move {
            if let Err(e) = sync_universe(universe, token, server, ws_url, http).await {
                warn!("universe sync exited: {e:#}");
            }
        }));
    }
    futures_util::future::join_all(handles).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-universe sync
// ---------------------------------------------------------------------------

async fn sync_universe(
    universe: UniverseMapping,
    token: String,
    server: String,
    ws_url: String,
    http: Arc<reqwest::Client>,
) -> Result<()> {
    let root = PathBuf::from(&universe.local);
    let slug = universe.slug.clone();

    info!("{slug}: starting — {}", root.display());

    // ── 1. Initial full push via REST ─────────────────────────────────────
    info!("{slug}: initial push…");
    let (md_count, bin_count) = initial_push(&root, &slug, &server, &token, &http).await;
    info!("{slug}: initial push done — {md_count} text, {bin_count} binary");

    // Reindex so the server reflects the full push immediately.
    let _ = http
        .post(format!("{server}/api/v1/universes/{slug}/reindex"))
        .bearer_auth(&token)
        .send()
        .await;

    // ── 2. WebSocket watcher for ongoing .md changes ──────────────────────
    let (bin_tx, mut bin_rx) = mpsc::unbounded_channel::<PathBuf>();
    let root2 = root.clone();
    let slug2 = slug.clone();

    // Spawn the binary-file watcher on a dedicated thread (notify needs it).
    std::thread::spawn(move || {
        watch_binaries_blocking(root2, slug2, bin_tx);
    });

    // Spawn REST uploader for binary changes.
    let http2 = Arc::clone(&http);
    let server2 = server.clone();
    let token2 = token.clone();
    let slug2 = slug.clone();
    let root2 = root.clone();
    tokio::spawn(async move {
        // Debounce: collect events for 500 ms before uploading.
        let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
        loop {
            tokio::select! {
                Some(p) = bin_rx.recv() => { pending.insert(p, Instant::now()); }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    let now = Instant::now();
                    let ready: Vec<PathBuf> = pending
                        .iter()
                        .filter(|(_, t)| now.duration_since(**t) >= Duration::from_millis(400))
                        .map(|(p, _)| p.clone())
                        .collect();
                    for path in ready {
                        pending.remove(&path);
                        upload_binary(&path, &root2, &slug2, &server2, &token2, &http2).await;
                    }
                }
            }
        }
    });

    // ── 3. WebSocket watcher for .md files (reconnects automatically) ─────
    loop {
        let cfg = WatcherConfig {
            watch_dirs: vec![root.clone()],
            universe_key: slug.clone(),
            server_url: ws_url.clone(),
            auth_token: token.clone(),
            resume_token: 0,
        };
        match SyncWatcher::new(cfg).run().await {
            Ok(()) => info!("{slug}: WS disconnected, reconnecting…"),
            Err(e) => warn!("{slug}: WS error: {e:#}, reconnecting…"),
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

// ---------------------------------------------------------------------------
// Initial full push (REST)
// ---------------------------------------------------------------------------

async fn initial_push(
    root: &Path,
    slug: &str,
    server: &str,
    token: &str,
    http: &reqwest::Client,
) -> (usize, usize) {
    let mut md = 0usize;
    let mut bin = 0usize;

    let entries = walkdir(root);
    for path in entries {
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if ext == "md" || ext == "yaml" || ext == "yml" || ext == "toml" {
            // Text file → Vault PUT
            if let Ok(body) = std::fs::read_to_string(&path) {
                let url = format!("{server}/api/v1/universes/{slug}/vault/{rel}");
                let _ = http.put(&url).bearer_auth(token).body(body).send().await;
                md += 1;
            }
        } else if is_binary_ext(&ext) {
            // Binary file → asset upload
            upload_binary(&path, root, slug, server, token, http).await;
            bin += 1;
        }
    }
    (md, bin)
}

// ---------------------------------------------------------------------------
// Binary file upload via REST assets endpoint
// ---------------------------------------------------------------------------

async fn upload_binary(
    path: &Path,
    root: &Path,
    slug: &str,
    server: &str,
    token: &str,
    http: &reqwest::Client,
) {
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");

    let Ok(bytes) = std::fs::read(path) else {
        return;
    };

    // Skip if already uploaded (compare sha256 against asset index).
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let check_url = format!("{server}/api/v1/universes/{slug}/assets/{sha256}");
    if http
        .get(&check_url)
        .bearer_auth(token)
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
    {
        return; // already on server
    }

    let url = format!("{server}/api/v1/universes/{slug}/assets?filename={filename}");
    match http
        .post(&url)
        .bearer_auth(token)
        .header("Content-Type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            info!("{slug}: uploaded {} → sha256:{}…", filename, &sha256[..12]);
            // If a sibling .md reference card mentions this file, update it.
            maybe_update_reference_card(path, root, slug, server, token, &sha256, http).await;
        }
        Ok(r) => warn!("{slug}: asset upload {} → {}", filename, r.status()),
        Err(e) => warn!("{slug}: asset upload {filename} error: {e}"),
    }
}

/// If a sibling `.md` file has `file: <filename>` and no `blob_sha256`,
/// inject the sha256, push the updated card.
async fn maybe_update_reference_card(
    bin_path: &Path,
    root: &Path,
    slug: &str,
    server: &str,
    token: &str,
    sha256: &str,
    http: &reqwest::Client,
) {
    let stem = bin_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let filename = bin_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let u = CardUpdate {
        root,
        slug,
        server,
        token,
        sha256,
        filename,
        http,
    };
    let card = bin_path.with_extension("md");
    if card.exists() {
        update_card(&card, &u).await;
    } else {
        let dir = bin_path.parent().unwrap_or(root);
        let alt = dir.join(format!("{stem}.md"));
        if alt.exists() {
            update_card(&alt, &u).await;
        }
    }
}

struct CardUpdate<'a> {
    root: &'a Path,
    slug: &'a str,
    server: &'a str,
    token: &'a str,
    sha256: &'a str,
    filename: &'a str,
    http: &'a reqwest::Client,
}

async fn update_card(card: &Path, u: &CardUpdate<'_>) {
    let Ok(text) = std::fs::read_to_string(card) else {
        return;
    };
    if !text.contains(&format!("file: {}", u.filename)) || text.contains("blob_sha256:") {
        return;
    }
    let updated = text.replacen(
        &format!("file: {}", u.filename),
        &format!("file: {}\nblob_sha256: {}", u.filename, u.sha256),
        1,
    );
    if std::fs::write(card, &updated).is_ok() {
        let rel = card
            .strip_prefix(u.root)
            .unwrap_or(card)
            .to_string_lossy()
            .replace('\\', "/");
        let url = format!("{}/api/v1/universes/{}/vault/{rel}", u.server, u.slug);
        let _ = u
            .http
            .put(&url)
            .bearer_auth(u.token)
            .body(updated)
            .send()
            .await;
        info!("{}: updated {rel} with blob_sha256", u.slug);
    }
}

// ---------------------------------------------------------------------------
// Binary filesystem watcher (blocking thread)
// ---------------------------------------------------------------------------

fn watch_binaries_blocking(root: PathBuf, slug: String, tx: mpsc::UnboundedSender<PathBuf>) {
    let (raw_tx, raw_rx) = std::sync::mpsc::channel();
    let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(ev) = res {
            let _ = raw_tx.send(ev);
        }
    }) {
        Ok(w) => w,
        Err(e) => {
            warn!("{slug}: binary watcher error: {e}");
            return;
        }
    };

    if let Err(e) = watcher.watch(&root, RecursiveMode::Recursive) {
        warn!("{slug}: watch error: {e}");
        return;
    }

    while let Ok(ev) = raw_rx.recv() {
        for path in ev.paths {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if is_binary_ext(&ext) && !is_ignored(&path) {
                let _ = tx.send(path);
            }
        }
    }
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

fn detect_slug(dir: &Path) -> String {
    let manifest = dir.join("_universe.yaml");
    if let Ok(raw) = std::fs::read_to_string(&manifest) {
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("name:") {
                let name = rest.trim().trim_matches('"').trim_matches('\'');
                if !name.is_empty() {
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
    dir.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_lowercase()
}

fn save_config(config_path: &Path, token: &str, dirs: &[PathBuf], server: &str) -> Result<()> {
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
    config.save(config_path)
}

fn is_binary_ext(ext: &str) -> bool {
    matches!(
        ext,
        "pdf"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "mp4"
            | "webm"
            | "mp3"
            | "ogg"
            | "wav"
            | "m4a"
            | "epub"
    )
}

fn is_ignored(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with('.') || IGNORE_NAMES.iter().any(|n| *n == s.as_ref())
    })
}

/// Walk a directory recursively, skipping ignored paths, returning all files.
fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn recurse(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_ignored(&path) {
                continue;
            }
            if path.is_dir() {
                recurse(&path, out);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    recurse(root, &mut out);
    out
}
