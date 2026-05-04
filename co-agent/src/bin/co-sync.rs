//! `co-sync` — content universe sync daemon.
//!
//! Reads `co-universes.yaml` (searched upward from cwd).
//! For each declared universe: ensures it exists, sets parent, pushes all
//! local files, reindexes, then watches for changes.
//!
//! Usage:  co-sync <token>
//! Token:  co.artelonga.com.br/co/settings/sync  (one time)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use co_agent::sync_config::{SyncConfig, UniverseRegistry};
use co_agent::watcher::{SyncWatcher, WatcherConfig};
use notify::RecursiveMode;
use notify::Watcher as _;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const IGNORE_NAMES: &[&str] = &[
    ".DS_Store",
    ".git",
    ".svn",
    "node_modules",
    "target",
    ".co",
    "__pycache__",
];

#[derive(Parser)]
#[command(
    name = "co-sync",
    about = "Sync content universes — reads co-universes.yaml"
)]
struct Cli {
    /// API token from co.artelonga.com.br/co/settings/sync
    token: String,
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
    let cwd = std::env::current_dir()?;
    let registry_path = UniverseRegistry::find(&cwd).context("co-universes.yaml not found")?;
    let registry_dir = registry_path.parent().unwrap_or(&cwd).to_path_buf();
    info!("Registry: {}", registry_path.display());

    let registry = UniverseRegistry::load(&registry_path)?;
    save_token(&cli.token, &registry.server)?;

    let http = Arc::new(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?,
    );

    // Step 1: ensure all universes exist with correct parents
    for decl in &registry.universes {
        ensure_universe(decl, &registry.server, &cli.token, &http).await;
    }

    // Step 2: sync universes that have local content
    let syncable: Vec<_> = registry
        .universes
        .iter()
        .filter(|d| !d.server_only && d.local.is_some())
        .filter_map(|d| {
            UniverseRegistry::resolve_local(d, &registry_dir).map(|p| (d.slug.clone(), p))
        })
        .collect();

    if syncable.is_empty() {
        warn!("No local universes to sync.");
        return Ok(());
    }

    info!(
        "Syncing {} universe(s): {}",
        syncable.len(),
        syncable
            .iter()
            .map(|(s, _)| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let ws_url = format!(
        "{}/api/v1/sync/ws",
        registry
            .server
            .replace("https://", "wss://")
            .replace("http://", "ws://")
    );

    let mut handles = Vec::new();
    for (slug, root) in syncable {
        let token = cli.token.clone();
        let server = registry.server.clone();
        let ws_url = ws_url.clone();
        let http = Arc::clone(&http);
        handles.push(tokio::spawn(async move {
            if let Err(e) = sync_universe(slug, root, token, server, ws_url, http).await {
                warn!("sync exited: {e:#}");
            }
        }));
    }
    futures_util::future::join_all(handles).await;
    Ok(())
}

async fn ensure_universe(
    decl: &co_agent::sync_config::UniverseDecl,
    server: &str,
    token: &str,
    http: &reqwest::Client,
) {
    let url = format!("{server}/api/v1/universes/{}", decl.slug);
    let exists = http
        .get(&url)
        .send()
        .await
        .is_ok_and(|r| r.status().is_success());
    if !exists {
        let body = serde_json::json!({
            "key": decl.slug, "name": decl.name, "description": decl.description,
        });
        match http
            .post(format!("{server}/api/v1/universes"))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() || r.status().as_u16() == 409 => {}
            Ok(r) => {
                warn!("create '{}': {}", decl.slug, r.status());
                return;
            }
            Err(e) => {
                warn!("create '{}': {e}", decl.slug);
                return;
            }
        }
    }
    // Set visibility + parent in one PUT
    let mut update = serde_json::json!({ "visibility": decl.visibility });
    if let Some(parent) = &decl.parent {
        update["parent_key"] = serde_json::Value::String(parent.clone());
    }
    let _ = http.put(&url).bearer_auth(token).json(&update).send().await;
    info!(
        "  {} ready (parent: {})",
        decl.slug,
        decl.parent.as_deref().unwrap_or("—")
    );
}

async fn sync_universe(
    slug: String,
    root: PathBuf,
    token: String,
    server: String,
    ws_url: String,
    http: Arc<reqwest::Client>,
) -> Result<()> {
    if !root.exists() {
        warn!("{slug}: '{}' does not exist — skipping", root.display());
        return Ok(());
    }
    info!("{slug}: {}", root.display());

    let (md, bin) = initial_push(&root, &slug, &server, &token, &http).await;
    info!("{slug}: {md} text + {bin} binary");

    let _ = http
        .post(format!("{server}/api/v1/universes/{slug}/reindex"))
        .bearer_auth(&token)
        .send()
        .await;
    info!("{slug}: reindexed ✓");

    // Binary watcher
    let (bin_tx, mut bin_rx) = mpsc::unbounded_channel::<PathBuf>();
    let (r2, s2, sl2) = (root.clone(), slug.clone(), slug.clone());
    std::thread::spawn(move || watch_binaries_blocking(r2, sl2, bin_tx));

    let (h2, sv2, tk2, sl2, rt2) = (
        Arc::clone(&http),
        server.clone(),
        token.clone(),
        slug.clone(),
        root.clone(),
    );
    tokio::spawn(async move {
        let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
        loop {
            tokio::select! {
                Some(p) = bin_rx.recv() => { pending.insert(p, Instant::now()); }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    let now = Instant::now();
                    let ready: Vec<_> = pending.iter()
                        .filter(|(_, t)| now.duration_since(**t) >= Duration::from_millis(400))
                        .map(|(p, _)| p.clone()).collect();
                    for path in ready {
                        pending.remove(&path);
                        upload_binary(&path, &rt2, &sl2, &sv2, &tk2, &h2).await;
                    }
                }
            }
        }
    });
    let _ = s2; // used in thread spawn above

    loop {
        let cfg = WatcherConfig {
            watch_dirs: vec![root.clone()],
            universe_key: slug.clone(),
            server_url: ws_url.clone(),
            auth_token: token.clone(),
            resume_token: 0,
        };
        match SyncWatcher::new(cfg).run().await {
            Ok(()) => info!("{slug}: WS reconnecting…"),
            Err(e) => warn!("{slug}: WS {e:#}, reconnecting…"),
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn initial_push(
    root: &Path,
    slug: &str,
    server: &str,
    token: &str,
    http: &reqwest::Client,
) -> (usize, usize) {
    let (mut md, mut bin) = (0usize, 0usize);
    for path in walkdir(root) {
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if matches!(ext.as_str(), "md" | "yaml" | "yml" | "toml") {
            if let Ok(body) = std::fs::read_to_string(&path) {
                let _ = http
                    .put(format!("{server}/api/v1/universes/{slug}/vault/{rel}"))
                    .bearer_auth(token)
                    .body(body)
                    .send()
                    .await;
                md += 1;
            }
        } else if is_binary_ext(&ext) {
            upload_binary(&path, root, slug, server, token, http).await;
            bin += 1;
        }
    }
    (md, bin)
}

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
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    if http
        .get(format!("{server}/api/v1/universes/{slug}/assets/{sha256}"))
        .bearer_auth(token)
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
    {
        return;
    }
    match http
        .post(format!(
            "{server}/api/v1/universes/{slug}/assets?filename={filename}"
        ))
        .bearer_auth(token)
        .header("Content-Type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            info!("{slug}: {filename} uploaded");
            update_ref_card(
                path,
                &RefCardCtx {
                    root,
                    slug,
                    server,
                    token,
                    sha256: &sha256,
                    filename,
                    http,
                },
            )
            .await;
        }
        _ => {}
    }
}

struct RefCardCtx<'a> {
    root: &'a Path,
    slug: &'a str,
    server: &'a str,
    token: &'a str,
    sha256: &'a str,
    filename: &'a str,
    http: &'a reqwest::Client,
}

async fn update_ref_card(bin: &Path, c: &RefCardCtx<'_>) {
    let stem = bin.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let card = {
        let card_inner = bin.with_extension("md");
        if card_inner.exists() {
            card_inner
        } else {
            let a = bin.parent().unwrap_or(c.root).join(format!("{stem}.md"));
            if !a.exists() {
                return;
            } else {
                a
            }
        }
    };
    let Ok(text) = std::fs::read_to_string(&card) else {
        return;
    };
    if !text.contains(&format!("file: {}", c.filename)) || text.contains("blob_sha256:") {
        return;
    }
    let updated = text.replacen(
        &format!("file: {}", c.filename),
        &format!("file: {}\nblob_sha256: {}", c.filename, c.sha256),
        1,
    );
    if std::fs::write(&card, &updated).is_ok() {
        let rel = card
            .strip_prefix(c.root)
            .unwrap_or(&card)
            .to_string_lossy()
            .replace('\\', "/");
        let _ = c
            .http
            .put(format!(
                "{}/api/v1/universes/{}/vault/{rel}",
                c.server, c.slug
            ))
            .bearer_auth(c.token)
            .body(updated)
            .send()
            .await;
    }
}

fn watch_binaries_blocking(root: PathBuf, slug: String, tx: mpsc::UnboundedSender<PathBuf>) {
    let (raw_tx, raw_rx) = std::sync::mpsc::channel();
    let Ok(mut watcher) = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(ev) = res {
            let _ = raw_tx.send(ev);
        }
    }) else {
        return;
    };
    if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
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
    let _ = slug;
}

fn save_token(token: &str, server: &str) -> Result<()> {
    let path = SyncConfig::default_path();
    let mut cfg = SyncConfig::load(&path).unwrap_or(SyncConfig {
        server: server.to_string(),
        ws_url: server
            .replace("https://", "wss://")
            .replace("http://", "ws://")
            + "/api/v1/sync/ws",
        token: token.to_string(),
        universes: vec![],
    });
    cfg.token = token.to_string();
    cfg.save(&path)
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
