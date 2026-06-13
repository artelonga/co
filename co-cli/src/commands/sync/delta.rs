//! CO-91 — `co sync` canonical content-author workflow.
//!
//! Promotes `scripts/seed-prod-universes.sh` into a first-class CLI surface.
//! An author edits markdown locally and runs `co sync push`; only the files
//! changed since the last push (detected via [jujutsu]) flow to the configured
//! deployment's Vault REST API, leaving a changelog paper trail.
//!
//! ## Primitives (1:1 with the script it replaces)
//!
//! - **Auth** — a per-deployment API token in the OS keychain (service `co`,
//!   account = the deployment's `token_name`), the same store `dev/co-token`
//!   and `--bootstrap` write. Read on demand; never persisted to disk here.
//! - **Delta** — the source repo is wrapped with `jj git init --colocate`
//!   (non-destructive; coexists with git). The last-pushed commit id is the
//!   baseline at `~/.co/sync-state/<deployment>/<universe>.commit`; only files
//!   differing between that baseline and `@` are uploaded.
//! - **Changelog** — `jj log -r '<baseline>..@'` is rendered to
//!   `~/.co/sync-runs/<deployment>/<universe>-<ts>.md` per run.
//! - **Per-deployment isolation** — tokens, baselines and changelog snippets
//!   are namespaced by deployment key, so `--to prod` and `--to uat` are
//!   independent operations against independent baselines.
//!
//! The orchestration is decoupled from `jj`, the keychain and HTTP via the
//! [`DeltaSource`], [`TokenReader`] and [`Uploader`] traits, so the planning
//! logic (which files, which changelog) is unit-tested in-process.
//!
//! [jujutsu]: https://github.com/jj-vcs/jj

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use colored::Colorize;
use serde::Deserialize;

const USER_AGENT: &str = concat!("co-cli/", env!("CARGO_PKG_VERSION"));
/// Zero-config fallback when `~/.co/deployments.toml` is absent — mirrors the
/// `PROD` constant of the script this command replaces.
const DEFAULT_PROD_URL: &str = "https://co-artelonga.fly.dev";

// ─── Options structs (filled from the CLI layer) ────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct PushOpts {
    /// Target deployment key (`--to`); `None` → the `default = true` entry.
    pub deployment: Option<String>,
    /// Universe slug override (`--universe`); `None` → from `.co/sync.toml`.
    pub universe: Option<String>,
    /// Source repo root (`--root`); `None` → current directory.
    pub root: Option<PathBuf>,
    pub full: bool,
    pub dry_run: bool,
    pub no_changelog: bool,
    pub bootstrap: bool,
    pub push_changelog: bool,
}

#[derive(Debug, Default, Clone)]
pub struct WatchOpts {
    pub deployment: Option<String>,
    pub universe: Option<String>,
    pub root: Option<PathBuf>,
    /// Debounce window in seconds (default 1).
    pub interval: Option<u64>,
}

#[derive(Debug, Default, Clone)]
pub struct ChangelogOpts {
    pub deployment: Option<String>,
    /// Filter to one universe (`--for`).
    pub universe: Option<String>,
}

// ─── `~/.co` layout ─────────────────────────────────────────────────────────

/// Root of CO's per-user state (`~/.co`, overridable with `$CO_HOME` for tests).
fn co_home() -> PathBuf {
    if let Ok(d) = std::env::var("CO_HOME") {
        return PathBuf::from(d);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".co")
}

fn deployments_path() -> PathBuf {
    co_home().join("deployments.toml")
}

/// `~/.co/sync-state/<deployment>/<universe>.commit`.
fn baseline_path(deployment: &str, universe: &str) -> PathBuf {
    co_home()
        .join("sync-state")
        .join(deployment)
        .join(format!("{universe}.commit"))
}

/// `~/.co/sync-runs/<deployment>/`.
fn runs_dir(deployment: &str) -> PathBuf {
    co_home().join("sync-runs").join(deployment)
}

fn read_baseline(deployment: &str, universe: &str) -> Option<String> {
    let raw = std::fs::read_to_string(baseline_path(deployment, universe)).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn save_baseline(deployment: &str, universe: &str, commit: &str) -> Result<()> {
    let path = baseline_path(deployment, universe);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&path, commit.as_bytes()).with_context(|| format!("writing {}", path.display()))
}

// ─── Deployments config (`~/.co/deployments.toml`) ──────────────────────────

#[derive(Debug, Deserialize)]
struct DeploymentEntry {
    url: String,
    token_name: Option<String>,
    #[serde(default)]
    default: bool,
}

/// A fully-resolved deployment target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployment {
    pub key: String,
    pub url: String,
    pub token_name: String,
}

fn parse_deployments(raw: &str) -> Result<BTreeMap<String, DeploymentEntry>> {
    toml::from_str(raw).context("parsing deployments.toml")
}

/// Load `~/.co/deployments.toml`, falling back to a built-in `prod` entry (the
/// script's zero-config behaviour) when the file is absent or empty.
fn load_deployments() -> Result<BTreeMap<String, DeploymentEntry>> {
    let path = deployments_path();
    match std::fs::read_to_string(&path) {
        Ok(raw) if !raw.trim().is_empty() => parse_deployments(&raw),
        _ => {
            let mut map = BTreeMap::new();
            map.insert(
                "prod".to_string(),
                DeploymentEntry {
                    url: DEFAULT_PROD_URL.to_string(),
                    token_name: Some("prod".to_string()),
                    default: true,
                },
            );
            Ok(map)
        }
    }
}

/// Pick the active deployment: `requested` → the `default = true` entry → a
/// lone `prod` entry. Errors when ambiguous.
fn resolve_deployment(
    map: &BTreeMap<String, DeploymentEntry>,
    requested: Option<&str>,
) -> Result<Deployment> {
    let key = match requested {
        Some(k) => {
            if !map.contains_key(k) {
                bail!(
                    "unknown deployment '{k}'. Known: {}",
                    map.keys().cloned().collect::<Vec<_>>().join(", ")
                );
            }
            k.to_string()
        }
        None => map
            .iter()
            .find(|(_, d)| d.default)
            .map(|(k, _)| k.clone())
            .or_else(|| map.contains_key("prod").then(|| "prod".to_string()))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no default deployment in {}. Mark one `default = true` or pass --to.",
                    deployments_path().display()
                )
            })?,
    };
    let entry = &map[&key];
    Ok(Deployment {
        token_name: entry.token_name.clone().unwrap_or_else(|| key.clone()),
        url: entry.url.trim_end_matches('/').to_string(),
        key,
    })
}

// ─── Per-repo config (`<root>/.co/sync.toml`) ───────────────────────────────

#[derive(Debug, Default, Deserialize)]
struct SyncToml {
    universe: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

/// A resolved push target: which universe, which directory, which files.
struct Target {
    universe: String,
    root: PathBuf,
    filter: FileFilter,
}

fn resolve_target(universe: Option<String>, root: Option<PathBuf>) -> Result<Target> {
    let root = match root {
        Some(r) => r,
        None => std::env::current_dir().context("resolving current directory")?,
    };
    let cfg_path = root.join(".co").join("sync.toml");
    let cfg: SyncToml = match std::fs::read_to_string(&cfg_path) {
        Ok(raw) => {
            toml::from_str(&raw).with_context(|| format!("parsing {}", cfg_path.display()))?
        }
        Err(_) => SyncToml::default(),
    };
    let universe = universe.or(cfg.universe).ok_or_else(|| {
        anyhow::anyhow!(
            "no universe configured. Add `universe = \"…\"` to {} or pass --universe.",
            cfg_path.display()
        )
    })?;
    let filter = FileFilter::new(cfg.include, cfg.exclude);
    Ok(Target {
        universe,
        root,
        filter,
    })
}

// ─── Include / exclude globbing ─────────────────────────────────────────────

/// Glob filter for the upload set. Defaults to `**/*.md` minus the usual build
/// and tooling directories — the same exclusions the script hard-coded.
pub struct FileFilter {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl FileFilter {
    fn new(include: Option<Vec<String>>, exclude: Option<Vec<String>>) -> Self {
        let include = include
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec!["**/*.md".to_string()]);
        let mut exclude = exclude.unwrap_or_default();
        for d in [
            "node_modules",
            "build",
            "dist",
            "target",
            ".git",
            ".jj",
            ".next",
            ".svelte-kit",
            ".claude",
            ".obsidian",
            ".cache",
            ".vercel",
            ".venv",
            "__pycache__",
            "seed-co",
        ] {
            exclude.push(format!("**/{d}/**"));
            exclude.push(format!("{d}/**"));
        }
        Self { include, exclude }
    }

    pub fn matches(&self, path: &str) -> bool {
        self.include.iter().any(|p| glob_match(p, path))
            && !self.exclude.iter().any(|p| glob_match(p, path))
    }

    /// Filter, de-duplicate and sort a candidate path list.
    fn apply(&self, paths: Vec<String>) -> Vec<String> {
        let mut out: Vec<String> = paths.into_iter().filter(|p| self.matches(p)).collect();
        out.sort();
        out.dedup();
        out
    }
}

/// Match a `/`-segmented glob supporting `*` (within a segment), `?` (one
/// char) and `**` (zero or more whole segments).
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').collect();
    let t: Vec<&str> = path.split('/').collect();
    seg_match(&p, &t)
}

fn seg_match(p: &[&str], t: &[&str]) -> bool {
    match p.first() {
        None => t.is_empty(),
        Some(&"**") => (0..=t.len()).any(|i| seg_match(&p[1..], &t[i..])),
        Some(seg) => match t.first() {
            Some(first) if wildcard_match(seg.as_bytes(), first.as_bytes()) => {
                seg_match(&p[1..], &t[1..])
            }
            _ => false,
        },
    }
}

/// Within-segment wildcard match (`*` = any run not crossing `/`, `?` = one).
fn wildcard_match(pat: &[u8], text: &[u8]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_p, mut star_t): (Option<usize>, usize) = (None, 0);
    while ti < text.len() {
        if pi < pat.len() && (pat[pi] == b'?' || pat[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star_p = Some(pi);
            star_t = ti;
            pi += 1;
        } else if let Some(sp) = star_p {
            pi = sp + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

// ─── Delta source (jujutsu) ─────────────────────────────────────────────────

/// The version-control delta surface the engine depends on. Implemented by
/// [`JjSource`] in production and by a fake in tests.
pub trait DeltaSource {
    /// Commit id of the current working copy (`@`).
    fn current(&self) -> Result<String>;
    /// Whether `baseline` still resolves in history (false → repo rewritten).
    fn baseline_valid(&self, baseline: &str) -> Result<bool>;
    /// Paths changed between `baseline` and `@` (repo-root-relative).
    fn changed(&self, baseline: &str) -> Result<Vec<String>>;
    /// Every tracked file (for a full upload / first run).
    fn all_files(&self) -> Result<Vec<String>>;
    /// Rendered `jj log -r '<baseline>..@'` snippet.
    fn log(&self, baseline: &str) -> Result<String>;
}

/// `jj`-backed [`DeltaSource`]. Shells out, mirroring the script exactly.
pub struct JjSource {
    root: PathBuf,
}

impl JjSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn jj(&self, args: &[&str]) -> Result<std::process::Output> {
        Command::new("jj")
            .args(args)
            .current_dir(&self.root)
            .output()
            .context("running `jj` — is jujutsu installed? (https://github.com/jj-vcs/jj)")
    }

    fn jj_ok(&self, args: &[&str]) -> Result<String> {
        let out = self.jj(args)?;
        if !out.status.success() {
            bail!(
                "jj {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// Wrap the repo with jj if it is not already (`jj git init --colocate`
    /// when a git repo is present, else a plain `jj git init`).
    pub fn ensure_init(&self) -> Result<()> {
        if self.root.join(".jj").is_dir() {
            return Ok(());
        }
        eprintln!(
            "  {} initializing jj wrapper in {} …",
            "→".blue(),
            self.root.display()
        );
        let args: &[&str] = if self.root.join(".git").exists() {
            &["git", "init", "--colocate"]
        } else {
            &["git", "init"]
        };
        self.jj_ok(args).map(|_| ())
    }
}

impl DeltaSource for JjSource {
    fn current(&self) -> Result<String> {
        let out = self.jj_ok(&["log", "-r", "@", "--no-graph", "-T", "commit_id"])?;
        Ok(out.lines().next().unwrap_or("").trim().to_string())
    }

    fn baseline_valid(&self, baseline: &str) -> Result<bool> {
        Ok(self
            .jj(&["log", "-r", baseline, "--no-graph", "-T", "commit_id"])?
            .status
            .success())
    }

    fn changed(&self, baseline: &str) -> Result<Vec<String>> {
        let out = self.jj_ok(&["diff", "--from", baseline, "--to", "@", "--name-only"])?;
        Ok(out
            .lines()
            .map(|l| l.trim().replace('\\', "/"))
            .filter(|l| !l.is_empty())
            .collect())
    }

    fn all_files(&self) -> Result<Vec<String>> {
        // `jj file list` enumerates every tracked path at `@`.
        let out = self.jj_ok(&["file", "list"])?;
        Ok(out
            .lines()
            .map(|l| l.trim().replace('\\', "/"))
            .filter(|l| !l.is_empty())
            .collect())
    }

    fn log(&self, baseline: &str) -> Result<String> {
        let template = "change_id.short() ++ \" \" ++ \
             author.timestamp().format(\"%Y-%m-%d %H:%M\") ++ \" — \" ++ \
             description.first_line() ++ \"\\n\"";
        self.jj_ok(&[
            "log",
            "-r",
            &format!("{baseline}..@"),
            "--no-graph",
            "-T",
            template,
        ])
    }
}

/// Compute the upload set: full when forced, on the first run, or when the
/// baseline was lost; otherwise the `baseline..@` delta. Returns the filtered
/// path list plus whether it was a full upload.
fn plan_files(
    source: &dyn DeltaSource,
    baseline: Option<&str>,
    full: bool,
    filter: &FileFilter,
) -> Result<(Vec<String>, bool)> {
    let use_full = full
        || match baseline {
            None => true,
            Some(b) => !source.baseline_valid(b)?,
        };
    if use_full {
        return Ok((filter.apply(source.all_files()?), true));
    }
    let changed = source.changed(baseline.expect("baseline present in delta path"))?;
    Ok((filter.apply(changed), false))
}

// ─── Token reader (OS keychain) ─────────────────────────────────────────────

/// Reads a deployment's API token. [`KeyringReader`] hits the OS keychain;
/// tests inject a fake.
pub trait TokenReader {
    fn read(&self, name: &str) -> Result<String>;
}

pub struct KeyringReader;

impl TokenReader for KeyringReader {
    fn read(&self, name: &str) -> Result<String> {
        // Same store as `dev/co-token` and `--bootstrap`: service "co".
        let entry = keyring::Entry::new("co", name)
            .with_context(|| format!("opening keychain entry co/{name}"))?;
        match entry.get_password() {
            Ok(t) => Ok(t),
            Err(keyring::Error::NoEntry) => bail!(
                "no '{name}' token in the OS keychain. \
                 Run `co sync push --bootstrap` once to create and store one."
            ),
            Err(e) => Err(e).with_context(|| format!("reading keychain entry co/{name}")),
        }
    }
}

fn store_token(name: &str, value: &str) -> Result<()> {
    let entry = keyring::Entry::new("co", name)
        .with_context(|| format!("opening keychain entry co/{name}"))?;
    entry
        .set_password(value)
        .with_context(|| format!("storing token in keychain entry co/{name}"))
}

// ─── Vault uploader (HTTP) ──────────────────────────────────────────────────

/// The upload surface: content-negotiated file PUTs plus an optional changelog
/// event POST. [`HttpUploader`] talks to a CO server; tests use a fake.
pub trait Uploader {
    /// Verify the credential works (vault listing). Best-effort; surfaces a
    /// clear error before a long upload loop.
    fn verify(&self) -> Result<()>;
    /// Upload one file, negotiating the `.co` protobuf wire format if the
    /// server advertises it (else `text/markdown`).
    fn upload(&self, path: &str, content: &str) -> Result<()>;
    /// POST a synthesized `event` entry carrying the run's changelog (CO-89).
    fn post_event(&self, path: &str, markdown: &str) -> Result<()>;
}

pub struct HttpUploader {
    base_url: String,
    universe: String,
    bearer: String,
    client: reqwest::blocking::Client,
    /// Lazily-probed protobuf support (`None` until first negotiated).
    protobuf: std::cell::Cell<Option<bool>>,
}

impl HttpUploader {
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
            protobuf: std::cell::Cell::new(None),
        }
    }

    fn vault_base(&self) -> String {
        format!(
            "{}/api/v1/universes/{}/vault/",
            self.base_url, self.universe
        )
    }

    fn vault_url(&self, path: &str) -> String {
        format!("{}{}", self.vault_base(), encode_path(path))
    }

    /// Probe `OPTIONS /…/vault/` once; treat any `co/protobuf` / `vnd.co`
    /// advertisement (in the `Accept-Post`/`Allow` headers or body) as support.
    fn supports_protobuf(&self) -> bool {
        if let Some(v) = self.protobuf.get() {
            return v;
        }
        let supported = self
            .client
            .request(reqwest::Method::OPTIONS, self.vault_base())
            .header("Authorization", format!("Bearer {}", self.bearer))
            .send()
            .ok()
            .map(|resp| {
                let advertises = |h: &str| {
                    resp.headers()
                        .get(h)
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.contains("vnd.co") || s.contains("co/protobuf"))
                        .unwrap_or(false)
                };
                advertises("accept-post") || advertises("allow") || advertises("accept")
            })
            .unwrap_or(false);
        self.protobuf.set(Some(supported));
        supported
    }

    fn put_markdown(&self, path: &str, content: &str) -> Result<()> {
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

    /// Send one file as a single-delta `.co` protobuf batch (CO-86/CO-151).
    fn put_cofile(&self, path: &str, content: &str) -> Result<()> {
        let sha = co::entry::Entry::hash_body(content);
        let delta = co::sync::delta::upserted_delta(
            self.universe.clone(),
            path.to_string(),
            content.as_bytes().to_vec(),
            sha,
            0,
        );
        let batch = co::sync::delta::Batch {
            deltas: vec![delta],
            client_id: USER_AGENT.to_string(),
            batch_ts_ns: 0,
            resume_token: 0,
        };
        let bytes = co::sync::delta::encode_batch(&batch).context("encoding .co batch")?;
        let resp = self
            .client
            .put(self.vault_url(path))
            .header("Authorization", format!("Bearer {}", self.bearer))
            .header(reqwest::header::CONTENT_TYPE, "application/vnd.co+protobuf")
            .body(bytes)
            .send()
            .with_context(|| format!("PUT vault/{path} (.co)"))?;
        if !resp.status().is_success() {
            bail!("PUT vault/{path} (.co) — HTTP {}", resp.status());
        }
        Ok(())
    }
}

impl Uploader for HttpUploader {
    fn verify(&self) -> Result<()> {
        let resp = self
            .client
            .get(self.vault_base())
            .header("Authorization", format!("Bearer {}", self.bearer))
            .send()
            .context("GET vault listing")?;
        match resp.status().as_u16() {
            200 => Ok(()),
            404 => bail!(
                "universe '{}' does not exist on the server yet — run `co sync push --bootstrap`.",
                self.universe
            ),
            code => bail!("token rejected (HTTP {code}). Re-run `co sync push --bootstrap`."),
        }
    }

    fn upload(&self, path: &str, content: &str) -> Result<()> {
        if self.supports_protobuf() {
            self.put_cofile(path, content)
        } else {
            self.put_markdown(path, content)
        }
    }

    fn post_event(&self, path: &str, markdown: &str) -> Result<()> {
        // CO-89 integration via the Vault PUT fast-path: the server parses the
        // frontmatter `type: event` and indexes it on the calendar view.
        self.put_markdown(path, markdown)
    }
}

/// Percent-encode each path segment (keeping `/` separators), matching the
/// script's `urllib.parse.quote(seg, safe='')`.
fn encode_path(path: &str) -> String {
    path.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    for b in seg.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ─── Changelog rendering ────────────────────────────────────────────────────

/// Render a per-run changelog snippet from the jj log between baseline and `@`.
fn render_changelog(
    universe: &str,
    root: &Path,
    baseline: Option<&str>,
    current: &str,
    log: &str,
) -> String {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let mut out = String::new();
    out.push_str(&format!("# Upload run — {universe} — {now}\n\n"));
    out.push_str(&format!("- baseline: {}\n", baseline.unwrap_or("<none>")));
    out.push_str(&format!("- current:  {current}\n"));
    out.push_str(&format!("- source:   {}\n\n", root.display()));
    match baseline {
        None => out.push_str("## First upload (no commit history to summarize)\n"),
        Some(_) => {
            out.push_str("## Commits since last upload\n\n");
            if log.trim().is_empty() {
                out.push_str("(no commits — working-copy edits only)\n");
            } else {
                out.push_str(log.trim_end());
                out.push('\n');
            }
        }
    }
    out
}

/// Write the snippet to `~/.co/sync-runs/<deployment>/<universe>-<ts>.md`.
fn write_changelog(deployment: &str, universe: &str, body: &str) -> Result<PathBuf> {
    let dir = runs_dir(deployment);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = dir.join(format!("{universe}-{ts}.md"));
    std::fs::write(&path, body.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

// ─── Engine orchestration ───────────────────────────────────────────────────

/// `co sync push` — the full delta push pipeline (non-bootstrap).
pub fn run_push(opts: PushOpts) -> Result<()> {
    if opts.bootstrap {
        return bootstrap(&opts);
    }
    let deployments = load_deployments()?;
    let deployment = resolve_deployment(&deployments, opts.deployment.as_deref())?;
    let target = resolve_target(opts.universe.clone(), opts.root.clone())?;

    let source = JjSource::new(target.root.clone());
    source.ensure_init()?;

    // A dry-run plans and prints only — no keychain read, no network.
    if opts.dry_run {
        return push_with(&source, None, &deployment, &target, &opts);
    }

    let token = KeyringReader.read(&deployment.token_name)?;
    let uploader = HttpUploader::new(&deployment.url, &target.universe, &token);
    push_with(&source, Some(&uploader), &deployment, &target, &opts)
}

/// Push pipeline against injected `source`/`uploader` (so the orchestration is
/// testable without `jj`, the keychain or a network). `uploader` is `None` for
/// a dry-run, which returns before any upload.
fn push_with(
    source: &dyn DeltaSource,
    uploader: Option<&dyn Uploader>,
    deployment: &Deployment,
    target: &Target,
    opts: &PushOpts,
) -> Result<()> {
    let baseline = read_baseline(&deployment.key, &target.universe);
    let current = source.current()?;
    let (files, was_full) = plan_files(source, baseline.as_deref(), opts.full, &target.filter)?;

    println!(
        "{} {} → {} ({}, {} file(s))",
        "↑".blue().bold(),
        target.universe.cyan(),
        deployment.key.cyan(),
        if was_full { "full" } else { "delta" },
        files.len(),
    );

    if opts.dry_run {
        for f in &files {
            println!("  {f}");
        }
        println!(
            "{} dry-run: {} file(s) would be uploaded (nothing sent).",
            "✓".green().bold(),
            files.len()
        );
        return Ok(());
    }

    if files.is_empty() {
        println!(
            "{} Nothing to push (no changes since last run).",
            "✓".green().bold()
        );
        // Still advance the baseline so the next run diffs from here.
        save_baseline(&deployment.key, &target.universe, &current)?;
        return Ok(());
    }

    let uploader = uploader.expect("uploader is required for a non-dry-run push");
    uploader.verify()?;
    let (mut ok, mut fail) = (0usize, 0usize);
    for rel in &files {
        let abs = target.root.join(rel);
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(_) => continue, // deleted between plan and upload
        };
        match uploader.upload(rel, &content) {
            Ok(()) => ok += 1,
            Err(e) => {
                fail += 1;
                if fail <= 3 {
                    eprintln!("  {} {rel}: {e:#}", "✗".red());
                }
            }
        }
    }
    println!("{} {ok} uploaded, {fail} failed", "✓".green().bold());

    if !opts.no_changelog {
        let log = baseline
            .as_deref()
            .map(|b| source.log(b))
            .transpose()
            .unwrap_or(None)
            .unwrap_or_default();
        let body = render_changelog(
            &target.universe,
            &target.root,
            baseline.as_deref(),
            &current,
            &log,
        );
        let path = write_changelog(&deployment.key, &target.universe, &body)?;
        println!("  {} changelog → {}", "📝".blue(), path.display());

        if opts.push_changelog {
            let commits = body.lines().filter(|l| l.contains(" — ")).count();
            let event = synthesize_event(&target.universe, commits, &body);
            let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
            match uploader.post_event(&format!("events/sync-{ts}.md"), &event) {
                Ok(()) => println!("  {} pushed changelog as event entry", "✓".green()),
                Err(e) => eprintln!("  {} could not push event: {e:#}", "warning:".yellow()),
            }
        }
    }

    save_baseline(&deployment.key, &target.universe, &current)?;
    Ok(())
}

/// Build the `event` entry markdown (frontmatter the server reads as
/// `entry_type: event`, `kind: sync`, `summary: <count>`).
fn synthesize_event(universe: &str, commits: usize, changelog: &str) -> String {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    format!(
        "---\ntype: event\nkind: sync\nsummary: \"{commits} commit(s) synced\"\nuniverse: {universe}\ndate: {now}\n---\n\n{changelog}\n"
    )
}

/// `co sync status` — non-mutating diff of what a push would upload.
pub fn run_status(
    deployment: Option<String>,
    universe: Option<String>,
    root: Option<PathBuf>,
) -> Result<()> {
    let deployments = load_deployments()?;
    let deployment = resolve_deployment(&deployments, deployment.as_deref())?;
    let target = resolve_target(universe, root)?;
    let source = JjSource::new(target.root.clone());
    source.ensure_init()?;

    let baseline = read_baseline(&deployment.key, &target.universe);
    let (files, was_full) = plan_files(&source, baseline.as_deref(), false, &target.filter)?;
    if files.is_empty() {
        println!(
            "{} {} is in sync with {}.",
            "✓".green().bold(),
            target.universe.cyan(),
            deployment.key.cyan()
        );
        return Ok(());
    }
    println!(
        "{} {} change(s) to push to {} ({}):",
        files.len(),
        if was_full {
            "full upload —"
        } else {
            "delta —"
        }
        .dimmed(),
        deployment.key.cyan(),
        target.universe.cyan()
    );
    for f in &files {
        println!("  {} {f}", "~".yellow());
    }
    Ok(())
}

/// `co sync watch` — debounced auto-push on local edits (CO-91 Phase 3).
pub fn run_watch(opts: WatchOpts) -> Result<()> {
    use notify_debouncer_mini::{DebounceEventResult, new_debouncer, notify::RecursiveMode};

    let secs = opts.interval.unwrap_or(1).max(1);
    let target = resolve_target(opts.universe.clone(), opts.root.clone())?;
    let push_opts = PushOpts {
        deployment: opts.deployment.clone(),
        universe: opts.universe.clone(),
        root: opts.root.clone(),
        no_changelog: true,
        ..Default::default()
    };

    println!(
        "{} watching {} (auto-push every {secs}s on save, Ctrl-C to stop)",
        "👁".blue(),
        target.root.display()
    );

    let (tx, rx) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(
        std::time::Duration::from_secs(secs),
        move |res: DebounceEventResult| {
            if res.is_ok() {
                let _ = tx.send(());
            }
        },
    )
    .context("starting file watcher")?;
    debouncer
        .watcher()
        .watch(&target.root, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", target.root.display()))?;

    for _ in rx {
        if let Err(e) = run_push(push_opts.clone()) {
            eprintln!("{} {e:#}", "warning:".yellow().bold());
        }
    }
    Ok(())
}

/// `co sync changelog` — print accumulated run snippets for a deployment.
pub fn run_changelog(opts: ChangelogOpts) -> Result<()> {
    let deployments = load_deployments()?;
    let deployment = resolve_deployment(&deployments, opts.deployment.as_deref())?;
    let dir = runs_dir(&deployment.key);
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => Vec::new(),
    };
    entries.retain(|p| p.extension().map(|e| e == "md").unwrap_or(false));
    if let Some(u) = &opts.universe {
        let prefix = format!("{u}-");
        entries.retain(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&prefix))
                .unwrap_or(false)
        });
    }
    entries.sort();
    if entries.is_empty() {
        println!(
            "{} no changelog snippets yet for {}.",
            "·".dimmed(),
            deployment.key.cyan()
        );
        return Ok(());
    }
    for path in entries {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        println!("{}", format!("── {} ──", path.display()).dimmed());
        println!("{body}");
    }
    Ok(())
}

// ─── Bootstrap (login + create + full upload + token + keychain) ────────────

fn bootstrap(opts: &PushOpts) -> Result<()> {
    let deployments = load_deployments()?;
    let deployment = resolve_deployment(&deployments, opts.deployment.as_deref())?;
    let target = resolve_target(opts.universe.clone(), opts.root.clone())?;

    println!(
        "{} bootstrapping {} on {} ({})",
        "⚙".blue().bold(),
        target.universe.cyan(),
        deployment.key.cyan(),
        deployment.url
    );

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .user_agent(USER_AGENT)
        .build()
        .expect("HTTP client");

    // 1. Password login → session JWT (works as a bearer for the rest).
    print!("Email: ");
    use std::io::Write as _;
    std::io::stdout().flush().ok();
    let mut email = String::new();
    std::io::stdin().read_line(&mut email).ok();
    let email = email.trim().to_string();
    let password = rpassword::prompt_password("Password: ").context("reading password")?;

    println!("[1/5] login as {email} …");
    let resp = client
        .post(format!("{}/api/v1/auth/password-login", deployment.url))
        .json(&serde_json::json!({ "email": email, "password": password }))
        .send()
        .context("POST password-login")?;
    let jwt = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|c| {
            c.split(';')
                .find_map(|p| p.trim().strip_prefix("session=").filter(|v| !v.is_empty()))
                .map(str::to_string)
        });
    if !resp.status().is_success() {
        bail!("login failed (HTTP {})", resp.status());
    }
    let jwt = jwt.ok_or_else(|| anyhow::anyhow!("server returned no session cookie"))?;
    println!("  {} authenticated", "✓".green());

    let bearer = |req: reqwest::blocking::RequestBuilder| {
        req.header("Authorization", format!("Bearer {jwt}"))
    };

    // 2. Create the universe (idempotent — 409 is fine).
    println!("[2/5] create universe '{}' …", target.universe);
    let create = bearer(client.post(format!("{}/api/v1/universes", deployment.url)))
        .json(&serde_json::json!({
            "key": target.universe,
            "name": target.universe,
            "description": format!("{} (seeded via co sync)", target.universe),
        }))
        .send()
        .context("POST universes")?;
    match create.status().as_u16() {
        201 => println!("  {} created", "✓".green()),
        409 => println!("  {} already exists", "·".dimmed()),
        code => bail!(
            "create universe — HTTP {code}: {}",
            create.text().unwrap_or_default()
        ),
    }

    // 3. Full upload using the session JWT as bearer.
    println!("[3/5] full upload …");
    let source = JjSource::new(target.root.clone());
    source.ensure_init()?;
    let current = source.current()?;
    let files = target.filter.apply(source.all_files()?);
    let uploader = HttpUploader::new(&deployment.url, &target.universe, &jwt);
    let (mut ok, mut fail) = (0usize, 0usize);
    for rel in &files {
        let content = match std::fs::read_to_string(target.root.join(rel)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        match uploader.upload(rel, &content) {
            Ok(()) => ok += 1,
            Err(e) => {
                fail += 1;
                if fail <= 3 {
                    eprintln!("  {} {rel}: {e:#}", "✗".red());
                }
            }
        }
    }
    println!(
        "  {} {ok} uploaded, {fail} failed (of {})",
        "✓".green(),
        files.len()
    );

    // 4. Generate a long-lived API token for future delta runs.
    println!("[4/5] generate API token …");
    let tok: serde_json::Value =
        bearer(client.post(format!("{}/api/v1/auth/token", deployment.url)))
            .json(&serde_json::json!({ "name": format!("co-sync-{}", deployment.key) }))
            .send()
            .context("POST auth/token")?
            .json()
            .context("parsing token response")?;
    let raw = tok["token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("server returned no token"))?;

    // 5. Store it in the OS keychain under the deployment's token_name.
    println!(
        "[5/5] store token in OS keychain (co/{}) …",
        deployment.token_name
    );
    store_token(&deployment.token_name, raw)?;
    println!("  {} stored", "✓".green());

    if !opts.no_changelog {
        let body = render_changelog(&target.universe, &target.root, None, &current, "");
        let path = write_changelog(&deployment.key, &target.universe, &body)?;
        println!("  {} changelog → {}", "📝".blue(), path.display());
    }
    save_baseline(&deployment.key, &target.universe, &current)?;

    println!(
        "\n{} bootstrap complete. Future runs: {}",
        "✓".green().bold(),
        "co sync push".cyan()
    );
    Ok(())
}

#[cfg(test)]
mod delta_tests;
