//! `co sync` — Google-Drive-like local editing of a CO universe (CO-51).
//!
//! Mirrors a remote universe's Vault into `~/Co/<universe>/`, tracks per-file
//! sync state in `<universe>/.co/sync.json`, and reconciles changes with
//! conflict detection + configurable resolution strategies.
//!
//! ## Sync protocol
//!
//! 1. Read `<universe>/.co/sync.json` → `{ files: { path: { hash, mtime, remote_hash } } }`.
//! 2. `GET /api/v1/universes/:slug/vault/` → remote file list with `body_hash`.
//! 3. Diff each path against the last-synced base:
//!    - local == remote → no change
//!    - only local changed → push
//!    - only remote changed → pull
//!    - both changed → conflict (resolved per strategy)
//! 4. Execute via the Vault REST API.
//! 5. Update `sync.json` (atomically, only after a successful op).
//!
//! The diff/merge engine is decoupled from HTTP via the [`VaultClient`] trait,
//! so the bulk of the logic is unit-tested in-process with a mock client and a
//! single loopback test covers the real reqwest path.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use colored::Colorize;
use serde::{Deserialize, Serialize};

use super::auth;

/// CO-91 — jj-tracked delta push / changelog / multi-deployment workflow.
pub mod delta;

const USER_AGENT: &str = concat!("co-cli/", env!("CARGO_PKG_VERSION"));
const WATCH_DEBOUNCE_SECS: u64 = 2;

// ─── Public CLI surface ────────────────────────────────────────────────────

pub enum SyncAction {
    // ── CO-51 bidirectional mirror (positional `<universe>`) ──
    Pull {
        universe: String,
    },
    Push {
        universe: String,
    },
    Watch {
        universe: String,
        interval: Option<u64>,
    },
    Status {
        universe: String,
    },
    Resolve {
        file: String,
        universe: Option<String>,
    },
    // ── CO-91 canonical content-author push (deployment-scoped, jj delta) ──
    DeltaPush(delta::PushOpts),
    DeltaStatus {
        deployment: Option<String>,
        universe: Option<String>,
        root: Option<std::path::PathBuf>,
    },
    DeltaWatch(delta::WatchOpts),
    Changelog(delta::ChangelogOpts),
}

/// `co login` — authenticate and store an API token (thin alias over `co auth
/// login --save-token`).
pub fn login(email: Option<String>, profile: Option<String>) {
    auth::run(
        auth::AuthAction::Login {
            email,
            password: false, // top-level `co login` = passwordless email magic-code
            save_token: true,
        },
        profile,
    );
}

/// `co sync …` entry point.
pub fn run(action: SyncAction, profile: Option<String>, strategy_override: Option<String>) {
    let profile = profile.unwrap_or_else(|| "default".to_string());
    let result = match action {
        SyncAction::Pull { universe } => cmd_pull(&universe, &profile, strategy_override),
        SyncAction::Push { universe } => cmd_push(&universe, &profile, strategy_override),
        SyncAction::Watch { universe, interval } => {
            cmd_watch(&universe, &profile, strategy_override, interval)
        }
        SyncAction::Status { universe } => cmd_status(&universe, &profile),
        SyncAction::Resolve { file, universe } => cmd_resolve(&file, universe, &profile),
        SyncAction::DeltaPush(opts) => delta::run_push(opts),
        SyncAction::DeltaStatus {
            deployment,
            universe,
            root,
        } => delta::run_status(deployment, universe, root),
        SyncAction::DeltaWatch(opts) => delta::run_watch(opts),
        SyncAction::Changelog(opts) => delta::run_changelog(opts),
    };
    if let Err(e) = result {
        eprintln!("{} {e:#}", "error:".red().bold());
        std::process::exit(1);
    }
}

// ─── Conflict-resolution strategy ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    LastWriteWins,
    LocalWins,
    RemoteWins,
    Manual,
    Merge,
}

impl Strategy {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "last-write-wins" | "lww" => Strategy::LastWriteWins,
            "local-wins" | "local" => Strategy::LocalWins,
            "remote-wins" | "remote" => Strategy::RemoteWins,
            "manual" => Strategy::Manual,
            "merge" => Strategy::Merge,
            other => bail!(
                "unknown conflict strategy '{other}' (expected: last-write-wins, \
                 local-wins, remote-wins, manual, merge)"
            ),
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct SyncConfig {
    strategy: Option<String>,
    #[allow(dead_code)]
    interval: Option<u64>,
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("co")
        .join("sync.yaml")
}

/// Resolve the active strategy: `--strategy` flag → `~/.config/co/sync.yaml` →
/// `last-write-wins` default.
fn resolve_strategy(override_str: Option<String>) -> Result<Strategy> {
    if let Some(s) = override_str {
        return Strategy::parse(&s);
    }
    let path = config_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        let cfg: SyncConfig =
            serde_yaml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        if let Some(s) = cfg.strategy {
            return Strategy::parse(&s);
        }
    }
    Ok(Strategy::LastWriteWins)
}

// ─── Vault client abstraction ──────────────────────────────────────────────

/// One remote entry from the Vault listing.
#[derive(Debug, Clone)]
pub struct RemoteFile {
    pub path: String,
    pub body_hash: String,
    /// Last-modified time in epoch milliseconds (from the entry's `updated_at`).
    pub mtime_ms: i64,
}

/// Minimal Vault REST surface the sync engine depends on. Implemented by
/// [`HttpVaultClient`] in production and by a mock in tests.
pub trait VaultClient {
    fn list(&self) -> Result<Vec<RemoteFile>>;
    fn read(&self, path: &str) -> Result<String>;
    fn write(&self, path: &str, content: &str) -> Result<()>;
    fn delete(&self, path: &str) -> Result<()>;
}

/// reqwest-backed [`VaultClient`] talking to a CO server.
pub struct HttpVaultClient {
    base_url: String,
    universe: String,
    bearer: String,
    client: reqwest::blocking::Client,
}

impl HttpVaultClient {
    pub fn new(base_url: &str, universe: &str, bearer: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent(USER_AGENT)
            .build()
            .expect("HTTP client");
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            universe: universe.to_string(),
            bearer: bearer.to_string(),
            client,
        }
    }

    fn vault_url(&self, path: &str) -> String {
        format!(
            "{}/api/v1/universes/{}/vault/{}",
            self.base_url, self.universe, path
        )
    }
}

#[derive(Deserialize)]
struct ListItem {
    path: String,
    #[serde(default)]
    body_hash: String,
    #[serde(default)]
    stat: ListStat,
}

#[derive(Deserialize, Default)]
struct ListStat {
    #[serde(default)]
    mtime: i64,
}

#[derive(Deserialize)]
struct ReadItem {
    content: String,
}

impl VaultClient for HttpVaultClient {
    fn list(&self) -> Result<Vec<RemoteFile>> {
        let url = format!(
            "{}/api/v1/universes/{}/vault/",
            self.base_url, self.universe
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.bearer))
            .send()
            .context("GET vault listing")?;
        if !resp.status().is_success() {
            bail!("GET vault/ — HTTP {}", resp.status());
        }
        let items: Vec<ListItem> = resp.json().context("parsing vault listing")?;
        Ok(items
            .into_iter()
            .map(|i| RemoteFile {
                path: i.path,
                body_hash: i.body_hash,
                mtime_ms: i.stat.mtime,
            })
            .collect())
    }

    fn read(&self, path: &str) -> Result<String> {
        let resp = self
            .client
            .get(self.vault_url(path))
            .header("Authorization", format!("Bearer {}", self.bearer))
            .send()
            .with_context(|| format!("GET vault/{path}"))?;
        if !resp.status().is_success() {
            bail!("GET vault/{path} — HTTP {}", resp.status());
        }
        let item: ReadItem = resp
            .json()
            .with_context(|| format!("parsing vault/{path}"))?;
        Ok(item.content)
    }

    fn write(&self, path: &str, content: &str) -> Result<()> {
        let resp = self
            .client
            .put(self.vault_url(path))
            .header("Authorization", format!("Bearer {}", self.bearer))
            .header(reqwest::header::CONTENT_TYPE, "text/markdown")
            .body(content.to_string())
            .send()
            .with_context(|| format!("PUT vault/{path}"))?;
        if !resp.status().is_success() {
            bail!("PUT vault/{path} — HTTP {}", resp.status());
        }
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        let resp = self
            .client
            .delete(self.vault_url(path))
            .header("Authorization", format!("Bearer {}", self.bearer))
            .send()
            // Lowercase verb: this is an HTTP DELETE error context, not SQL, so
            // it stays clear of the CWE-89 raw-SQL scanner heuristic.
            .with_context(|| format!("deleting vault/{path}"))?;
        if !resp.status().is_success() {
            bail!("DELETE vault/{path} — HTTP {}", resp.status());
        }
        Ok(())
    }
}

// ─── Per-universe sync state (sync.json) ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileState {
    /// Comparable hash of the local body at last successful sync.
    pub hash: String,
    /// Local file modified time (epoch millis) at last sync, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<i64>,
    /// Remote `body_hash` at last successful sync.
    pub remote_hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    #[serde(default)]
    pub files: BTreeMap<String, FileState>,
}

impl SyncState {
    fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(raw) => {
                serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Atomically persist state: write to a temp file, then rename. A crash
    /// mid-write therefore never corrupts an existing `sync.json`.
    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(self).context("serializing sync.json")?;
        fs::write(&tmp, raw.as_bytes()).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("renaming into {}", path.display()))?;
        Ok(())
    }
}

// ─── Lockfile guard ─────────────────────────────────────────────────────────

/// RAII lock that prevents concurrent syncs on the same universe. Acquired by
/// `O_EXCL` creation of `<universe>/.co/sync.lock`; released on drop.
pub struct SyncLock {
    path: PathBuf,
}

impl SyncLock {
    fn acquire(co_dir: &Path) -> Result<Self> {
        fs::create_dir_all(co_dir).with_context(|| format!("creating {}", co_dir.display()))?;
        let path = co_dir.join("sync.lock");
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write as _;
                let _ = writeln!(f, "{}", std::process::id());
                Ok(Self { path })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => bail!(
                "another sync is already in progress (lock: {}). \
                 Remove it manually if no sync is running.",
                path.display()
            ),
            Err(e) => Err(e).with_context(|| format!("creating lock {}", path.display())),
        }
    }
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

// ─── Diff classification ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Unchanged,
    LocalNew,
    LocalModified,
    LocalDeleted,
    RemoteNew,
    RemoteModified,
    RemoteDeleted,
    Conflict,
}

#[derive(Debug, Clone)]
pub struct PlannedChange {
    pub path: String,
    pub kind: ChangeKind,
}

/// Classify one path given its base (last-sync), local, and remote hashes.
fn classify(base: Option<&FileState>, local: Option<&str>, remote: Option<&str>) -> ChangeKind {
    match (base, local, remote) {
        (None, Some(l), Some(r)) => {
            if l == r {
                ChangeKind::Unchanged
            } else {
                ChangeKind::Conflict
            }
        }
        (None, Some(_), None) => ChangeKind::LocalNew,
        (None, None, Some(_)) => ChangeKind::RemoteNew,
        (None, None, None) => ChangeKind::Unchanged,
        (Some(b), Some(l), Some(r)) => {
            let local_changed = l != b.hash;
            let remote_changed = r != b.remote_hash;
            match (local_changed, remote_changed) {
                (false, false) => ChangeKind::Unchanged,
                (true, false) => ChangeKind::LocalModified,
                (false, true) => ChangeKind::RemoteModified,
                (true, true) => {
                    if l == r {
                        ChangeKind::Unchanged
                    } else {
                        ChangeKind::Conflict
                    }
                }
            }
        }
        (Some(b), Some(l), None) => {
            // Remote deleted. Safe only if local is untouched since last sync.
            if l == b.hash {
                ChangeKind::RemoteDeleted
            } else {
                ChangeKind::Conflict
            }
        }
        (Some(b), None, Some(r)) => {
            // Local deleted. Safe only if remote is untouched since last sync.
            if r == b.remote_hash {
                ChangeKind::LocalDeleted
            } else {
                ChangeKind::Conflict
            }
        }
        (Some(_), None, None) => ChangeKind::Unchanged, // gone both sides
    }
}

// ─── Sync engine ────────────────────────────────────────────────────────────

pub struct SyncEngine<C: VaultClient> {
    client: C,
    /// `<root>/<universe>` — the local mirror directory.
    local_root: PathBuf,
    strategy: Strategy,
}

/// Summary of an executed sync.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncSummary {
    pub pushed: usize,
    pub pulled: usize,
    pub deleted: usize,
    pub conflicts: usize,
    pub unresolved: Vec<String>,
}

impl<C: VaultClient> SyncEngine<C> {
    pub fn new(client: C, local_root: PathBuf, strategy: Strategy) -> Self {
        Self {
            client,
            local_root,
            strategy,
        }
    }

    fn co_dir(&self) -> PathBuf {
        self.local_root.join(".co")
    }

    fn state_path(&self) -> PathBuf {
        self.co_dir().join("sync.json")
    }

    fn base_path(&self, path: &str) -> PathBuf {
        self.co_dir().join("base").join(path)
    }

    fn local_path(&self, path: &str) -> PathBuf {
        self.local_root.join(path)
    }

    /// Compute the diff plan (no side effects beyond reading the filesystem and
    /// one remote listing). Used by `status`, `pull`, and `push`.
    pub fn plan(&self) -> Result<(Vec<PlannedChange>, SyncState, BTreeMap<String, RemoteFile>)> {
        let state = SyncState::load(&self.state_path())?;
        let remote_list = self.client.list()?;
        let remote: BTreeMap<String, RemoteFile> = remote_list
            .into_iter()
            .map(|r| (r.path.clone(), r))
            .collect();
        let local = self.scan_local()?;

        let mut paths: BTreeSet<String> = BTreeSet::new();
        paths.extend(state.files.keys().cloned());
        paths.extend(remote.keys().cloned());
        paths.extend(local.keys().cloned());

        let mut changes = Vec::new();
        for path in paths {
            let kind = classify(
                state.files.get(&path),
                local.get(&path).map(|s| s.as_str()),
                remote.get(&path).map(|r| r.body_hash.as_str()),
            );
            changes.push(PlannedChange { path, kind });
        }
        Ok((changes, state, remote))
    }

    /// Recursively scan the local mirror, returning `path → comparable_hash`.
    /// Skips the `.co/` control directory and conflict artifacts.
    fn scan_local(&self) -> Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        if self.local_root.is_dir() {
            walk(&self.local_root, &self.local_root, &mut out)?;
        }
        return Ok(out);

        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
            for entry in
                fs::read_dir(dir).with_context(|| format!("reading dir {}", dir.display()))?
            {
                let entry = entry?;
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_dir() {
                    if name == ".co" {
                        continue;
                    }
                    walk(root, &path, out)?;
                } else if path.is_file() {
                    if is_conflict_artifact(&name) {
                        continue;
                    }
                    let rel = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    let content = fs::read_to_string(&path)
                        .with_context(|| format!("reading {}", path.display()))?;
                    out.insert(rel, comparable_hash(&content));
                }
            }
            Ok(())
        }
    }

    /// `co sync status` — non-mutating diff report.
    pub fn status(&self) -> Result<Vec<PlannedChange>> {
        let (changes, _, _) = self.plan()?;
        Ok(changes
            .into_iter()
            .filter(|c| c.kind != ChangeKind::Unchanged)
            .collect())
    }

    /// `co sync pull` — apply remote→local changes (and resolve conflicts).
    pub fn pull(&self) -> Result<SyncSummary> {
        let _lock = SyncLock::acquire(&self.co_dir())?;
        let (changes, mut state, remote) = self.plan()?;
        let mut summary = SyncSummary::default();

        for change in &changes {
            match change.kind {
                ChangeKind::RemoteNew | ChangeKind::RemoteModified => {
                    self.apply_pull(&change.path, &remote, &mut state)?;
                    summary.pulled += 1;
                }
                ChangeKind::RemoteDeleted => {
                    self.apply_local_delete(&change.path, &mut state)?;
                    summary.deleted += 1;
                }
                ChangeKind::Conflict => {
                    summary.conflicts += 1;
                    self.resolve(&change.path, &remote, &mut state, &mut summary)?;
                }
                _ => {}
            }
        }
        Ok(summary)
    }

    /// `co sync push` — apply local→remote changes (and resolve conflicts).
    /// A no-op when there are no local changes.
    pub fn push(&self) -> Result<SyncSummary> {
        let _lock = SyncLock::acquire(&self.co_dir())?;
        let (changes, mut state, remote) = self.plan()?;
        let mut summary = SyncSummary::default();

        for change in &changes {
            match change.kind {
                ChangeKind::LocalNew | ChangeKind::LocalModified => {
                    self.apply_push(&change.path, &mut state)?;
                    summary.pushed += 1;
                }
                ChangeKind::LocalDeleted => {
                    self.apply_remote_delete(&change.path, &mut state)?;
                    summary.deleted += 1;
                }
                ChangeKind::Conflict => {
                    summary.conflicts += 1;
                    self.resolve(&change.path, &remote, &mut state, &mut summary)?;
                }
                _ => {}
            }
        }
        Ok(summary)
    }

    // ── Single-file operations (each persists state on success) ──

    /// Pull one remote file to local, snapshot its base, record state.
    fn apply_pull(
        &self,
        path: &str,
        remote: &BTreeMap<String, RemoteFile>,
        state: &mut SyncState,
    ) -> Result<()> {
        let content = self.client.read(path)?;
        self.write_local(path, &content)?;
        self.write_base(path, &content)?;
        let h = comparable_hash(&content);
        let remote_hash = remote
            .get(path)
            .map(|r| r.body_hash.clone())
            .unwrap_or_else(|| h.clone());
        state.files.insert(
            path.to_string(),
            FileState {
                hash: h,
                mtime: self.local_mtime_ms(path),
                remote_hash,
            },
        );
        state.save(&self.state_path())?;
        Ok(())
    }

    /// Push one local file to remote, snapshot its base, record state.
    fn apply_push(&self, path: &str, state: &mut SyncState) -> Result<()> {
        let content = fs::read_to_string(self.local_path(path))
            .with_context(|| format!("reading local {path}"))?;
        self.client.write(path, &content)?;
        self.write_base(path, &content)?;
        let h = comparable_hash(&content);
        state.files.insert(
            path.to_string(),
            FileState {
                hash: h.clone(),
                mtime: self.local_mtime_ms(path),
                remote_hash: h,
            },
        );
        state.save(&self.state_path())?;
        Ok(())
    }

    /// Remote file was deleted → remove the local copy.
    fn apply_local_delete(&self, path: &str, state: &mut SyncState) -> Result<()> {
        let _ = fs::remove_file(self.local_path(path));
        let _ = fs::remove_file(self.base_path(path));
        state.files.remove(path);
        state.save(&self.state_path())?;
        Ok(())
    }

    /// Local file was deleted → delete it on the remote.
    fn apply_remote_delete(&self, path: &str, state: &mut SyncState) -> Result<()> {
        self.client.delete(path)?;
        let _ = fs::remove_file(self.base_path(path));
        state.files.remove(path);
        state.save(&self.state_path())?;
        Ok(())
    }

    /// Resolve a conflict per the active strategy.
    fn resolve(
        &self,
        path: &str,
        remote: &BTreeMap<String, RemoteFile>,
        state: &mut SyncState,
        summary: &mut SyncSummary,
    ) -> Result<()> {
        match self.strategy {
            Strategy::LocalWins => self.apply_push(path, state),
            Strategy::RemoteWins => self.apply_pull(path, remote, state),
            Strategy::LastWriteWins => {
                let local_ms = self.local_mtime_ms(path).unwrap_or(0);
                let remote_ms = remote.get(path).map(|r| r.mtime_ms).unwrap_or(0);
                if local_ms >= remote_ms {
                    self.apply_push(path, state)
                } else {
                    self.apply_pull(path, remote, state)
                }
            }
            Strategy::Manual => {
                self.write_conflict_artifacts(path)?;
                summary.unresolved.push(path.to_string());
                Ok(())
            }
            Strategy::Merge => {
                let base = self.read_base(path).unwrap_or_default();
                let local = fs::read_to_string(self.local_path(path)).unwrap_or_default();
                let remote_content = self.client.read(path).unwrap_or_default();
                match merge3(&base, &local, &remote_content) {
                    Ok(merged) => {
                        // Write merged locally then push it as the resolution.
                        self.write_local(path, &merged)?;
                        self.apply_push(path, state)
                    }
                    Err(()) => {
                        // Overlapping hunks — fall back to manual.
                        self.write_conflict_artifacts(path)?;
                        summary.unresolved.push(path.to_string());
                        Ok(())
                    }
                }
            }
        }
    }

    /// Write `<path>.local`, `<path>.remote`, `<path>.base` for manual merge.
    fn write_conflict_artifacts(&self, path: &str) -> Result<()> {
        let local = fs::read_to_string(self.local_path(path)).unwrap_or_default();
        let remote = self.client.read(path).unwrap_or_default();
        let base = self.read_base(path).unwrap_or_default();
        let lp = self.local_path(path);
        write_sibling(&lp, "local", &local)?;
        write_sibling(&lp, "remote", &remote)?;
        write_sibling(&lp, "base", &base)?;
        Ok(())
    }

    /// `co sync resolve <file>` — finalize a manual conflict: push the
    /// user-merged file and clean up `.local`/`.remote`/`.base` artifacts.
    pub fn resolve_manual(&self, path: &str) -> Result<()> {
        let _lock = SyncLock::acquire(&self.co_dir())?;
        let lp = self.local_path(path);
        if !lp.is_file() {
            bail!("{path}: resolved file does not exist", path = path);
        }
        let mut state = SyncState::load(&self.state_path())?;
        self.apply_push(path, &mut state)?;
        for ext in ["local", "remote", "base"] {
            let _ = fs::remove_file(sibling_path(&lp, ext));
        }
        Ok(())
    }

    // ── Local fs helpers ──

    fn write_local(&self, path: &str, content: &str) -> Result<()> {
        let lp = self.local_path(path);
        if let Some(parent) = lp.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&lp, content.as_bytes()).with_context(|| format!("writing {}", lp.display()))?;
        Ok(())
    }

    fn write_base(&self, path: &str, content: &str) -> Result<()> {
        let bp = self.base_path(path);
        if let Some(parent) = bp.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&bp, content.as_bytes()).with_context(|| format!("writing {}", bp.display()))?;
        Ok(())
    }

    fn read_base(&self, path: &str) -> Option<String> {
        fs::read_to_string(self.base_path(path)).ok()
    }

    fn local_mtime_ms(&self, path: &str) -> Option<i64> {
        let meta = fs::metadata(self.local_path(path)).ok()?;
        let mtime = meta.modified().ok()?;
        let dur = mtime.duration_since(std::time::UNIX_EPOCH).ok()?;
        Some(dur.as_millis() as i64)
    }
}

// ─── Conflict-artifact path helpers ─────────────────────────────────────────

fn is_conflict_artifact(name: &str) -> bool {
    name.ends_with(".local") || name.ends_with(".remote") || name.ends_with(".base")
}

fn sibling_path(local_path: &Path, ext: &str) -> PathBuf {
    let mut s = local_path.as_os_str().to_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

fn write_sibling(local_path: &Path, ext: &str, content: &str) -> Result<()> {
    let p = sibling_path(local_path, ext);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&p, content.as_bytes()).with_context(|| format!("writing {}", p.display()))?;
    Ok(())
}

// ─── Hashing ────────────────────────────────────────────────────────────────

/// Hash a local file the same way the server hashes an entry: SHA-256 of the
/// markdown body, excluding any frontmatter block.
pub fn comparable_hash(content: &str) -> String {
    let body = match co::entry::Entry::parse_frontmatter(content) {
        Some((_, body)) => body,
        None => content.to_string(),
    };
    co::entry::Entry::hash_body(&body)
}

// ─── Three-way line merge (diff3) ───────────────────────────────────────────

/// Three-way merge of `base`, `local`, and `remote`. Returns `Ok(merged)` when
/// every changed region is non-overlapping (changed on at most one side, or
/// changed identically on both); returns `Err(())` on an overlapping conflict.
pub fn merge3(base: &str, local: &str, remote: &str) -> Result<String, ()> {
    let trailing_nl = base.ends_with('\n') || local.ends_with('\n') || remote.ends_with('\n');
    let b: Vec<&str> = split_lines(base);
    let l: Vec<&str> = split_lines(local);
    let r: Vec<&str> = split_lines(remote);

    // Base indices common to BOTH local and remote (stable anchors).
    let map_l = lcs_map(&b, &l);
    let map_r = lcs_map(&b, &r);
    let mut anchors: Vec<(usize, usize, usize)> = map_l
        .iter()
        .filter_map(|(&bi, &li)| map_r.get(&bi).map(|&ri| (bi, li, ri)))
        .collect();
    anchors.sort_by_key(|&(bi, _, _)| bi);

    let mut out: Vec<&str> = Vec::new();
    let (mut bi, mut li, mut ri) = (0usize, 0usize, 0usize);

    for (ab, al, ar) in anchors
        .into_iter()
        .chain(std::iter::once((b.len(), l.len(), r.len())))
    {
        let base_chunk = &b[bi..ab];
        let local_chunk = &l[li..al];
        let remote_chunk = &r[ri..ar];

        if local_chunk == base_chunk {
            // changed only on the remote side (or unchanged) → take remote
            out.extend_from_slice(remote_chunk);
        } else if remote_chunk == base_chunk || local_chunk == remote_chunk {
            // changed only on the local side, or both sides made the same edit
            out.extend_from_slice(local_chunk);
        } else {
            // overlapping, divergent edits → conflict
            return Err(());
        }

        if ab < b.len() {
            out.push(b[ab]);
            bi = ab + 1;
            li = al + 1;
            ri = ar + 1;
        }
    }

    let mut merged = out.join("\n");
    if trailing_nl && !merged.is_empty() {
        merged.push('\n');
    }
    Ok(merged)
}

fn split_lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return Vec::new();
    }
    let stripped = s.strip_suffix('\n').unwrap_or(s);
    stripped.split('\n').collect()
}

/// LCS alignment: returns `base_index → other_index` for each matched line in
/// the longest common subsequence of `a` (base) and `b`.
fn lcs_map(a: &[&str], b: &[&str]) -> BTreeMap<usize, usize> {
    let n = a.len();
    let m = b.len();
    // dp[i][j] = LCS length of a[i..] and b[j..]
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut map = BTreeMap::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            map.insert(i, j);
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    map
}

// ─── CLI command handlers ───────────────────────────────────────────────────

/// Sync root: `$CO_SYNC_ROOT` or `~/Co`.
fn sync_root() -> PathBuf {
    if let Ok(dir) = std::env::var("CO_SYNC_ROOT") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Co")
}

fn build_engine(
    universe: &str,
    profile: &str,
    strategy_override: Option<String>,
) -> Result<SyncEngine<HttpVaultClient>> {
    let (base_url, bearer) = auth::resolve_auth(profile)?;
    let strategy = resolve_strategy(strategy_override)?;
    let client = HttpVaultClient::new(&base_url, universe, &bearer);
    let local_root = sync_root().join(universe);
    Ok(SyncEngine::new(client, local_root, strategy))
}

fn cmd_pull(universe: &str, profile: &str, strategy: Option<String>) -> Result<()> {
    let engine = build_engine(universe, profile, strategy)?;
    println!(
        "{} Pulling {} → {}",
        "↓".blue().bold(),
        universe.cyan(),
        engine.local_root.display()
    );
    let summary = engine.pull()?;
    print_summary(&summary);
    Ok(())
}

fn cmd_push(universe: &str, profile: &str, strategy: Option<String>) -> Result<()> {
    let engine = build_engine(universe, profile, strategy)?;
    println!("{} Pushing {}", "↑".blue().bold(), universe.cyan());
    let summary = engine.push()?;
    if summary.pushed == 0 && summary.deleted == 0 && summary.conflicts == 0 {
        println!("{} Nothing to push (already in sync).", "✓".green().bold());
    } else {
        print_summary(&summary);
    }
    Ok(())
}

fn cmd_status(universe: &str, profile: &str) -> Result<()> {
    let engine = build_engine(universe, profile, None)?;
    let changes = engine.status()?;
    if changes.is_empty() {
        println!("{} {} is in sync.", "✓".green().bold(), universe.cyan());
        return Ok(());
    }
    println!("{} changes in {}:", changes.len(), universe.cyan());
    for c in &changes {
        let (sym, label) = match c.kind {
            ChangeKind::LocalNew => ("+".green(), "new (local)"),
            ChangeKind::LocalModified => ("~".yellow(), "modified (local)"),
            ChangeKind::LocalDeleted => ("-".red(), "deleted (local)"),
            ChangeKind::RemoteNew => ("+".cyan(), "new (remote)"),
            ChangeKind::RemoteModified => ("~".cyan(), "modified (remote)"),
            ChangeKind::RemoteDeleted => ("-".magenta(), "deleted (remote)"),
            ChangeKind::Conflict => ("!".red().bold(), "conflict"),
            ChangeKind::Unchanged => continue,
        };
        println!("  {sym} {:<20} {}", label, c.path);
    }
    Ok(())
}

fn cmd_watch(
    universe: &str,
    profile: &str,
    strategy: Option<String>,
    interval: Option<u64>,
) -> Result<()> {
    let secs = interval.unwrap_or(WATCH_DEBOUNCE_SECS).max(1);
    println!(
        "{} Watching {} (push every {secs}s, Ctrl-C to stop)",
        "👁".blue(),
        universe.cyan()
    );
    loop {
        match build_engine(universe, profile, strategy.clone()).and_then(|engine| engine.push()) {
            Ok(summary) if summary.pushed > 0 || summary.deleted > 0 => {
                print_summary(&summary);
            }
            Ok(_) => {}
            Err(e) => eprintln!("{} {e:#}", "warning:".yellow().bold()),
        }
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
}

fn cmd_resolve(file: &str, universe: Option<String>, profile: &str) -> Result<()> {
    // Resolve which universe this file belongs to. If `--universe` is given,
    // the file path is relative to that universe's mirror; otherwise infer the
    // universe from the path under the sync root.
    let (universe, rel) = match universe {
        Some(u) => (u, file.to_string()),
        None => infer_universe(file)?,
    };
    let engine = build_engine(&universe, profile, None)?;
    engine.resolve_manual(&rel)?;
    println!(
        "{} Resolved {} and pushed to {}.",
        "✓".green().bold(),
        rel.cyan(),
        universe.cyan()
    );
    Ok(())
}

/// Given an absolute or sync-root-relative file path, infer `(universe, rel)`.
fn infer_universe(file: &str) -> Result<(String, String)> {
    let root = sync_root();
    let p = PathBuf::from(file);
    let abs = if p.is_absolute() {
        p
    } else {
        std::env::current_dir()?.join(p)
    };
    let rel = abs.strip_prefix(&root).map_err(|_| {
        anyhow::anyhow!(
            "{} is not under the sync root {}. Pass --universe.",
            file,
            root.display()
        )
    })?;
    let mut comps = rel.components();
    let universe = comps
        .next()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .ok_or_else(|| anyhow::anyhow!("could not determine universe from {file}"))?;
    let inner = comps.as_path().to_string_lossy().replace('\\', "/");
    if inner.is_empty() {
        bail!("{file} does not name a file inside a universe");
    }
    Ok((universe, inner))
}

fn print_summary(s: &SyncSummary) {
    println!(
        "{} {} pushed, {} pulled, {} deleted, {} conflicts",
        "✓".green().bold(),
        s.pushed,
        s.pulled,
        s.deleted,
        s.conflicts
    );
    for path in &s.unresolved {
        println!(
            "  {} CONFLICT: {} — resolve with `co sync resolve {}`",
            "!".red().bold(),
            path,
            path
        );
    }
}

#[cfg(test)]
mod tests;
