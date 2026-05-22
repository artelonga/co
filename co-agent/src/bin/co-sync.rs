//! `co-sync` — content universe sync daemon + CI push tool.
//!
//! Reads `co-universes.yaml` (searched upward from cwd).
//!
//! **Daemon mode** (`co-sync <token>`):
//!   For each declared universe: ensures it exists, sets parent, pushes all
//!   local files, reindexes, then watches for changes via WebSocket.
//!
//! **Push mode** (`co-sync push --config co-universes.yaml`, token from `CO_TOKEN`):
//!   One-shot CI push. Reads `syncs:` entries, walks source folders, skips
//!   files whose SHA-256 hash is unchanged (cached in `~/.co/push-cache.json`),
//!   PUTs changed files to the CO Vault API, then exits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use co_agent::sync_config::{SyncConfig, SyncEntry, UniverseRegistry};
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
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// API token for daemon mode (co.artelonga.com.br/co/settings/sync)
    token: Option<String>,
}

#[derive(Subcommand)]
enum Cmd {
    /// One-shot CI push: reads `syncs:` from co-universes.yaml and PUTs
    /// changed files to the CO Vault API. Skips unchanged files (SHA-256 cache).
    Push {
        /// CO Vault API token (or set CO_TOKEN env var)
        #[arg(env = "CO_TOKEN")]
        token: String,
        /// Path to co-universes.yaml (default: search upward from cwd)
        #[arg(long)]
        config: Option<PathBuf>,
    },
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

    match cli.cmd {
        Some(Cmd::Push { token, config }) => {
            let cwd = std::env::current_dir()?;
            let registry_path = match config {
                Some(p) => p,
                None => UniverseRegistry::find(&cwd)
                    .context("co-universes.yaml not found (searched upward from cwd)")?,
            };
            let registry = UniverseRegistry::load(&registry_path)?;
            let registry_dir = registry_path.parent().unwrap_or(&cwd).to_path_buf();
            return run_push(&registry, &registry_dir, &token).await;
        }
        None => {}
    }

    let token = cli.token.context(
        "token required in daemon mode — usage: co-sync <token>\n\
         For CI push mode use: co-sync push (CO_TOKEN env var)",
    )?;

    let cwd = std::env::current_dir()?;
    let registry_path = UniverseRegistry::find(&cwd).context("co-universes.yaml not found")?;
    let registry_dir = registry_path.parent().unwrap_or(&cwd).to_path_buf();
    info!("Registry: {}", registry_path.display());

    let registry = UniverseRegistry::load(&registry_path)?;
    save_token(&token, &registry.server)?;

    let http = Arc::new(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?,
    );

    // Step 1: ensure all universes exist with correct parents
    for decl in &registry.universes {
        ensure_universe(decl, &registry.server, &token, &http).await;
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
        let token = token.clone();
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

// ---------------------------------------------------------------------------
// push mode — one-shot CI sync
// ---------------------------------------------------------------------------

/// Hash cache stored at `~/.co/push-cache.json`.
/// Maps `"{slug}/{rel_path}"` → SHA-256 hex of last-uploaded content.
type PushCache = HashMap<String, String>;

fn push_cache_path() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".co")
        .join("push-cache.json")
}

fn load_push_cache() -> PushCache {
    let p = push_cache_path();
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_push_cache(cache: &PushCache) {
    let p = push_cache_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(&p, json.as_bytes());
    }
}

/// Returns `true` if the file's content matches the cached SHA-256.
fn is_unchanged(cache: &PushCache, cache_key: &str, content: &[u8]) -> bool {
    let hash = format!("{:x}", Sha256::digest(content));
    cache.get(cache_key).is_some_and(|h| *h == hash)
}

fn current_hash(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

/// Returns true if `name` matches any of the glob patterns (e.g. `"*.md"`).
/// Empty pattern list → matches everything (include all).
fn matches_patterns(name: &str, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| glob_match(p, name))
}

/// Minimal glob matcher supporting `*` (any chars) and `?` (one char).
fn glob_match(pattern: &str, name: &str) -> bool {
    let mut p = pattern.chars().peekable();
    let mut n = name.chars().peekable();
    glob_match_inner(&mut p, &mut n)
}

fn glob_match_inner(
    p: &mut std::iter::Peekable<std::str::Chars<'_>>,
    n: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> bool {
    loop {
        match p.peek() {
            None => return n.peek().is_none(),
            Some('*') => {
                p.next();
                if p.peek().is_none() {
                    return true; // trailing `*` matches remainder
                }
                // try matching from every position in n
                let rest_p: String = p.clone().collect();
                let rest_n: String = n.clone().collect();
                for i in 0..=rest_n.len() {
                    let mut pp = rest_p.chars().peekable();
                    let mut nn = rest_n[i..].chars().peekable();
                    if glob_match_inner(&mut pp, &mut nn) {
                        return true;
                    }
                }
                return false;
            }
            Some('?') => {
                p.next();
                if n.next().is_none() {
                    return false;
                }
            }
            Some(&pc) => {
                p.next();
                match n.next() {
                    Some(nc) if nc == pc => {}
                    _ => return false,
                }
            }
        }
    }
}

/// One-shot push: walk each `syncs:` entry, skip unchanged files, PUT the rest.
async fn run_push(registry: &UniverseRegistry, registry_dir: &Path, token: &str) -> Result<()> {
    if registry.syncs.is_empty() {
        warn!("No `syncs:` entries in co-universes.yaml — nothing to push");
        return Ok(());
    }

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let mut cache = load_push_cache();
    let mut total_pushed = 0usize;
    let mut total_skipped = 0usize;

    for sync in &registry.syncs {
        let source = resolve_path(&sync.source, registry_dir);
        info!("push: {} → {}", source.display(), sync.target);

        let files = collect_sync_files(&source, sync);
        for (abs_path, rel_path) in files {
            let cache_key = format!("{}/{}", sync.target, rel_path);
            let Ok(content) = std::fs::read(&abs_path) else {
                warn!("push: cannot read {}", abs_path.display());
                continue;
            };
            if is_unchanged(&cache, &cache_key, &content) {
                total_skipped += 1;
                continue;
            }
            let url = format!(
                "{}/api/v1/universes/{}/vault/{}",
                registry.server, sync.target, rel_path
            );
            match http
                .put(&url)
                .bearer_auth(token)
                .body(content.clone())
                .send()
                .await
            {
                Ok(r) if r.status().is_success() || r.status().as_u16() == 200 => {
                    cache.insert(cache_key, current_hash(&content));
                    total_pushed += 1;
                    info!("push: ✓ {}/{}", sync.target, rel_path);
                }
                Ok(r) => {
                    warn!("push: {} {} → {}", sync.target, rel_path, r.status());
                }
                Err(e) => {
                    warn!("push: {} {} error: {e}", sync.target, rel_path);
                }
            }
        }
    }

    save_push_cache(&cache);
    info!(
        "push done: {} uploaded, {} skipped (unchanged)",
        total_pushed, total_skipped
    );
    Ok(())
}

/// Resolve a source path from a sync entry (relative to registry dir, or `~/`).
fn resolve_path(raw: &str, registry_dir: &Path) -> PathBuf {
    if raw.starts_with("~/") {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(raw.strip_prefix("~/").unwrap_or(raw))
    } else if std::path::Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        registry_dir.join(raw)
    }
}

/// Collect (abs_path, rel_path) pairs for a sync entry.
///
/// If `source` is a file, returns just that file.
/// If `source` is a directory, walks recursively applying include/exclude.
fn collect_sync_files(source: &Path, sync: &SyncEntry) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    if source.is_file() {
        let name = source.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if should_include(name, &sync.include, &sync.exclude) {
            out.push((source.to_path_buf(), name.to_string()));
        }
        return out;
    }
    collect_sync_dir(source, source, sync, &mut out);
    out
}

fn collect_sync_dir(root: &Path, dir: &Path, sync: &SyncEntry, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_ignored(&path) {
            continue;
        }
        if path.is_dir() {
            collect_sync_dir(root, &path, sync, out);
        } else if path.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !should_include(name, &sync.include, &sync.exclude) {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(root) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                out.push((path, rel_str));
            }
        }
    }
}

fn should_include(name: &str, include: &[String], exclude: &[String]) -> bool {
    // Exclude wins over include
    if !exclude.is_empty() && matches_patterns(name, exclude) {
        return false;
    }
    matches_patterns(name, include)
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
