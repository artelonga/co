//! `co source add github <owner/repo>` — ingest a GitHub repository into a CO
//! universe as a navigable hierarchical tree of entries (CO-417).
//!
//! A general `source: github` adapter. Given `owner/repo` (and an optional ref),
//! it fetches the repo (shallow `git clone`), walks its file tree, and
//! materializes each file as a CO entry under a target universe — **preserving
//! the folder hierarchy** so folders become tree nodes in the UI.
//!
//! Per-file parse:
//! - `.md`   → entry body directly.
//! - `.ipynb`→ parse the notebook JSON, render markdown cells as markdown and
//!   code cells as fenced ``` blocks (with language), in cell order.
//! - other   → asset-style entry linking the original path.
//!
//! Each entry carries provenance frontmatter consumed by CO-418's traceback:
//! - `source: github:<owner/repo>@<sha>`
//! - `source_path: <path/in/repo>`
//!
//! Re-sync is idempotent: the Vault PUTs converge, and a second import of the
//! same ref is a no-op for unchanged files. Optional `--delete-missing`
//! soft-reflects repo deletions.
//!
//! The **fetch** (network/git) and the **parse/materialize** (pure) are separate
//! units so the transform can be tested without a live fetch — see the tests
//! at the bottom, which use a tiny in-memory fixture tree.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use colored::Colorize;
use serde::Deserialize;

use crate::commands::auth;

const DEFAULT_BASE_URL: &str = "https://co.artelonga.com.br";

/// A file discovered in the source repo: its path relative to the repo root
/// and its raw text content. Binary/asset files keep `text = None`.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceFile {
    /// Path relative to the repo root, forward-slashed (e.g. `notebooks/a.ipynb`).
    pub rel_path: String,
    /// UTF-8 content if the file is text; `None` for binary/asset files.
    pub text: Option<String>,
}

/// A materialized CO entry ready to be written via the Vault API.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedEntry {
    /// Vault path under the universe (e.g. `content/notebooks/a.md`).
    pub vault_path: String,
    /// Full markdown document: provenance frontmatter + body.
    pub markdown: String,
}

#[derive(Debug, Deserialize)]
struct VaultEntry {
    path: String,
    /// CO-438: SHA-256 of the entry body (frontmatter excluded). Absent on
    /// older servers that predate the listing field — treated as "unknown",
    /// so those entries are always re-PUT (never wrongly skipped).
    #[serde(default)]
    body_hash: Option<String>,
}

/// CO-423: minimal shape of `GET /api/v1/universes/{key}` used to confirm the
/// universe **really** exists. The SPA serves HTTP 200 for unknown routes, so a
/// bare status check false-positives; a parsed `key` that matches is the real
/// signal.
#[derive(Debug, Deserialize)]
struct UniverseInfoProbe {
    key: Option<String>,
}

// ─── Entry point ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn run(
    owner_repo: String,
    universe: Option<String>,
    reference: Option<String>,
    remote: Option<String>,
    token: Option<String>,
    dry_run: bool,
    delete_missing: bool,
    parent: Option<String>,
    no_index: bool,
    throttle_ms: u64,
) {
    if let Err(e) = do_add_github(
        owner_repo,
        universe,
        reference,
        remote,
        token,
        dry_run,
        delete_missing,
        parent,
        no_index,
        throttle_ms,
    ) {
        eprintln!("{} {:#}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn do_add_github(
    owner_repo: String,
    universe: Option<String>,
    reference: Option<String>,
    remote: Option<String>,
    token: Option<String>,
    dry_run: bool,
    delete_missing: bool,
    parent: Option<String>,
    no_index: bool,
    throttle_ms: u64,
) -> Result<()> {
    let (owner, repo) = parse_owner_repo(&owner_repo)?;
    let universe_key = universe.unwrap_or_else(|| sanitize_key(&repo));

    let (base_url, api_token) = resolve_auth(remote, token)?;

    eprintln!(
        "Fetching {} (ref: {})...",
        format!("{owner}/{repo}").cyan().bold(),
        reference.as_deref().unwrap_or("default")
    );

    // 1. Fetch (network) — separated from parse/materialize for testability.
    let fetched = fetch_github(&owner, &repo, reference.as_deref())?;
    let sha = fetched.sha.clone();
    let files = collect_source_files(&fetched.clone_dir)?;
    eprintln!("Fetched {} files @ {}", files.len(), short_sha(&sha));

    // 2. Materialize (pure) — parse each file and stamp provenance frontmatter.
    let slug = format!("{owner}/{repo}");
    let mut entries = materialize(&files, &slug, &sha);

    // CO-423: ensure a navigable landing exists. If the repo didn't bring an
    // `index.md` (at the content root), synthesize a minimal one linking the
    // imported tree. `--no-index` disables; an existing index is never replaced.
    if !no_index && !has_index_entry(&entries) {
        entries.insert(0, synth_index(&entries, &slug, &repo));
    }

    if dry_run {
        eprintln!("{}", "dry-run — no writes will be made".yellow().bold());
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent("co-cli/source")
        .build()
        .context("build HTTP client")?;

    // 3. Ensure target universe exists (and re-parent if requested).
    ensure_universe(
        &client,
        &base_url,
        &api_token,
        &universe_key,
        &repo,
        parent.as_deref(),
        dry_run,
    )?;

    // 4. Push entries via Vault PUT — skipping entries already byte-identical
    //    on the server (CO-438 Bug 3). The vault returns 429 after a burst of
    //    PUTs; re-running used to re-PUT every entry from the start, burning the
    //    budget on already-synced files and never reaching the tail. We now
    //    fetch the server's per-entry body hashes once and push only what
    //    changed, so a rate-limited import resumes and a re-sync is near-free.
    let server_hashes = if dry_run {
        HashMap::new()
    } else {
        fetch_server_hashes(&client, &base_url, &api_token, &universe_key)?
    };
    let to_push = select_changed(&entries, &server_hashes);
    let skipped = entries.len() - to_push.len();
    eprintln!(
        "Importing {} entries into '{}'{}...",
        to_push.len(),
        universe_key,
        if skipped > 0 {
            format!(" ({skipped} unchanged, skipped)")
        } else {
            String::new()
        }
    );
    for e in &to_push {
        if dry_run {
            eprintln!("  import  {}", e.vault_path);
        } else {
            put_entry(
                &client,
                &base_url,
                &api_token,
                &universe_key,
                &e.vault_path,
                &e.markdown,
            )?;
            if throttle_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(throttle_ms));
            }
        }
    }

    // 5. Optional reconcile: soft-reflect repo deletions.
    if delete_missing {
        reconcile_deletions(
            &client,
            &base_url,
            &api_token,
            &universe_key,
            &entries,
            dry_run,
        )?;
    }

    if dry_run {
        eprintln!(
            "dry-run complete — {} entries would be imported from {}@{}",
            entries.len(),
            slug,
            short_sha(&sha)
        );
    } else {
        eprintln!(
            "{} {} — {} entries from {}@{}",
            "imported".green().bold(),
            universe_key,
            entries.len(),
            slug,
            short_sha(&sha)
        );
    }

    Ok(())
}

// ─── owner/repo parsing ────────────────────────────────────────────────────────

/// Parse `owner/repo` (tolerating a leading `github:` scheme or a full URL).
fn parse_owner_repo(input: &str) -> Result<(String, String)> {
    let s = input
        .trim()
        .trim_start_matches("github:")
        .trim_start_matches("https://github.com/")
        .trim_start_matches("git@github.com:")
        .trim_end_matches(".git")
        .trim_matches('/');

    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() != 2 {
        bail!("expected owner/repo, got '{input}'");
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

// ─── Fetch (network — not unit-tested) ─────────────────────────────────────────

/// A fetched repo on disk plus the resolved commit sha. `_tmp` is held only to
/// keep the temp dir alive until the caller is done collecting files.
struct Fetched {
    _tmp: tempfile::TempDir,
    clone_dir: PathBuf,
    sha: String,
}

/// Shallow-clone the repo into a temp dir. Public repos need no auth; a
/// `GITHUB_TOKEN` in the environment is injected for private repos / rate limits.
fn fetch_github(owner: &str, repo: &str, reference: Option<&str>) -> Result<Fetched> {
    let tmp = tempfile::tempdir().context("create temp dir for clone")?;
    let dest = tmp.path().join("repo");

    let url = match std::env::var("GITHUB_TOKEN") {
        Ok(tok) if !tok.is_empty() => {
            format!("https://x-access-token:{tok}@github.com/{owner}/{repo}.git")
        }
        _ => format!("https://github.com/{owner}/{repo}.git"),
    };

    let mut cmd = std::process::Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1");
    if let Some(r) = reference {
        cmd.arg("--branch").arg(r);
    }
    cmd.arg(&url).arg(&dest);
    let out = cmd.output().context("run git clone")?;
    if !out.status.success() {
        bail!(
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    let rev = std::process::Command::new("git")
        .arg("-C")
        .arg(&dest)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .context("run git rev-parse")?;
    if !rev.status.success() {
        bail!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&rev.stderr).trim()
        );
    }
    let sha = String::from_utf8_lossy(&rev.stdout).trim().to_string();

    Ok(Fetched {
        _tmp: tmp,
        clone_dir: dest,
        sha,
    })
}

// ─── Source-file collection ────────────────────────────────────────────────────

/// Walk a cloned repo dir and return its files (skipping `.git` and hidden
/// dotfiles). Paths are relative to `clone_dir`.
fn collect_source_files(clone_dir: &Path) -> Result<Vec<SourceFile>> {
    collect_from_dir(clone_dir)
}

/// Pure file collection from a real directory tree (used by `collect_source_files`
/// and directly by tests via a fixture dir). Hidden dotfiles/dirs (incl. `.git`)
/// are skipped; paths are forward-slashed and relative to `base`.
fn collect_from_dir(base: &Path) -> Result<Vec<SourceFile>> {
    let mut files = Vec::new();
    walk_recursive(base, base, &mut files)?;
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(files)
}

fn walk_recursive(base: &Path, dir: &Path, out: &mut Vec<SourceFile>) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry.context("read dir entry")?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // Skip hidden files/dirs (including .git).
        if name.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            walk_recursive(base, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path).ok();
            out.push(SourceFile {
                rel_path: rel,
                text,
            });
        }
    }
    Ok(())
}

// ─── Materialize (pure — the testable core) ───────────────────────────────────

/// Turn fetched source files into materialized CO entries, preserving the
/// folder hierarchy under `content/` and stamping provenance frontmatter.
///
/// `slug` is `owner/repo`; `sha` is the resolved commit. Pure & deterministic.
pub fn materialize(files: &[SourceFile], slug: &str, sha: &str) -> Vec<MaterializedEntry> {
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        out.push(materialize_one(f, slug, sha));
    }
    out
}

fn materialize_one(file: &SourceFile, slug: &str, sha: &str) -> MaterializedEntry {
    let kind = classify(&file.rel_path);
    let (vault_path, body) = match kind {
        FileKind::Markdown => {
            let body = file.text.clone().unwrap_or_default();
            (rewrite_ext(&file.rel_path, "md"), body)
        }
        FileKind::Notebook => {
            let body = file
                .text
                .as_deref()
                .map(ipynb_to_markdown)
                .unwrap_or_else(|| "_(could not read notebook)_".to_string());
            (rewrite_ext(&file.rel_path, "md"), body)
        }
        FileKind::Asset => {
            let body = asset_body(&file.rel_path, slug, sha);
            (rewrite_ext(&file.rel_path, "md"), body)
        }
    };

    let frontmatter = provenance_frontmatter(&file.rel_path, slug, sha, kind);
    let markdown = format!("{frontmatter}{body}");
    MaterializedEntry {
        vault_path: format!("content/{vault_path}"),
        markdown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    Markdown,
    Notebook,
    Asset,
}

fn classify(rel_path: &str) -> FileKind {
    let ext = Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" => FileKind::Markdown,
        "ipynb" => FileKind::Notebook,
        _ => FileKind::Asset,
    }
}

/// Replace the file extension while preserving the directory path.
fn rewrite_ext(rel_path: &str, new_ext: &str) -> String {
    let p = Path::new(rel_path);
    let parent = p.parent().map(|x| x.to_string_lossy().to_string());
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel_path.to_string());
    match parent {
        Some(dir) if !dir.is_empty() => format!("{dir}/{stem}.{new_ext}"),
        _ => format!("{stem}.{new_ext}"),
    }
}

/// Build the provenance frontmatter block (CO-418 consumes `source`/`source_path`).
fn provenance_frontmatter(rel_path: &str, slug: &str, sha: &str, kind: FileKind) -> String {
    let title = title_from_path(rel_path);
    let entry_type = match kind {
        FileKind::Notebook => "notebook",
        FileKind::Asset => "asset",
        FileKind::Markdown => "page",
    };
    format!(
        "---\n\
         title: {title}\n\
         source: github:{slug}@{sha}\n\
         source_path: {rel_path}\n\
         source_kind: github\n\
         entry_type: {entry_type}\n\
         ---\n\n"
    )
}

/// Asset entries link the original file at the pinned sha.
fn asset_body(rel_path: &str, slug: &str, sha: &str) -> String {
    let raw = format!("https://raw.githubusercontent.com/{slug}/{sha}/{rel_path}");
    let blob = format!("https://github.com/{slug}/blob/{sha}/{rel_path}");
    format!(
        "Asset imported from [`{rel_path}`]({blob}).\n\n\
         - Raw: [{raw}]({raw})\n"
    )
}

fn title_from_path(rel_path: &str) -> String {
    Path::new(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel_path.to_string())
}

// ─── Auto-index / landing (CO-423) ─────────────────────────────────────────────

/// True if the import already contains a content-root `index.md` (case-insensitive)
/// — in which case we never synthesize or overwrite one.
fn has_index_entry(entries: &[MaterializedEntry]) -> bool {
    entries
        .iter()
        .any(|e| e.vault_path.eq_ignore_ascii_case("content/index.md"))
}

/// Build a minimal navigable `content/index.md` landing that links each
/// imported top-level entry, so the universe opens with a curated entry point
/// instead of falling back to the SPA shell.
fn synth_index(entries: &[MaterializedEntry], slug: &str, repo: &str) -> MaterializedEntry {
    // Link the shallowest entries first (top-level files), then the rest, to
    // keep the landing readable. Paths are vault paths under `content/`.
    let mut links: Vec<String> = entries
        .iter()
        .map(|e| {
            let rel = e
                .vault_path
                .strip_prefix("content/")
                .unwrap_or(&e.vault_path);
            let label = title_from_path(rel);
            format!("- [{label}]({rel})")
        })
        .collect();
    links.sort();

    let body = if links.is_empty() {
        format!("Imported from [`{slug}`](https://github.com/{slug}).\n")
    } else {
        format!(
            "Imported from [`{slug}`](https://github.com/{slug}).\n\n## Conteúdo\n\n{}\n",
            links.join("\n")
        )
    };

    let frontmatter = format!(
        "---\n\
         title: {repo}\n\
         source_kind: github\n\
         entry_type: page\n\
         ---\n\n"
    );

    MaterializedEntry {
        vault_path: "content/index.md".to_string(),
        markdown: format!("{frontmatter}{body}"),
    }
}

// ─── ipynb → markdown (the key transform) ──────────────────────────────────────

/// Render a Jupyter notebook (`.ipynb` JSON) to readable markdown.
///
/// - `markdown` cells → emitted verbatim.
/// - `code` cells → fenced ``` blocks tagged with the notebook language.
/// - cell `source` may be a string or an array of line strings (Jupyter spec).
/// - empty cells are skipped.
/// - cells are emitted in document order.
///
/// On invalid JSON, returns the raw input wrapped so nothing is lost.
pub fn ipynb_to_markdown(json: &str) -> String {
    let nb: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return format!("```json\n{json}\n```\n"),
    };

    let lang = nb
        .get("metadata")
        .and_then(|m| m.get("kernelspec"))
        .and_then(|k| k.get("language"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            nb.get("metadata")
                .and_then(|m| m.get("language_info"))
                .and_then(|l| l.get("name"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("python")
        .to_string();

    let cells = match nb.get("cells").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return format!("```json\n{json}\n```\n"),
    };

    let mut parts: Vec<String> = Vec::new();
    for cell in cells {
        let cell_type = cell
            .get("cell_type")
            .and_then(|v| v.as_str())
            .unwrap_or("code");
        let src = join_source(cell.get("source"));
        if src.trim().is_empty() {
            continue;
        }
        match cell_type {
            "markdown" | "raw" => parts.push(src.trim_end().to_string()),
            _ => {
                // code (default)
                parts.push(format!("```{lang}\n{}\n```", src.trim_end()));
            }
        }
    }

    let mut body = parts.join("\n\n");
    if !body.is_empty() {
        body.push('\n');
    }
    body
}

/// Jupyter `source` is either a JSON string or an array of line strings.
fn join_source(source: Option<&serde_json::Value>) -> String {
    match source {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(lines)) => lines
            .iter()
            .filter_map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

// ─── Auth resolution (mirrors push.rs) ─────────────────────────────────────────

fn resolve_auth(remote: Option<String>, token: Option<String>) -> Result<(String, String)> {
    let base_url = remote
        .or_else(|| std::env::var("CO_REMOTE").ok())
        .or_else(|| {
            let creds = auth::load_credentials();
            creds.get("default").and_then(|p| p.base_url.clone())
        })
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

    let base_url = base_url.trim_end_matches('/').to_string();

    let api_token = token
        .or_else(|| std::env::var("CO_TOKEN").ok())
        .or_else(|| {
            let creds = auth::load_credentials();
            creds.get("default").and_then(|p| p.token.clone())
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no API token — pass --token, set CO_TOKEN, or run `co auth token create --save`"
            )
        })?;

    Ok((base_url, api_token))
}

// ─── Universe + Vault operations (mirrors push.rs) ─────────────────────────────

#[allow(clippy::too_many_arguments)]
fn ensure_universe(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    key: &str,
    name: &str,
    parent: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    if universe_exists(client, base_url, token, key)? {
        eprintln!("Universe '{key}' exists — importing into it");
    } else {
        eprintln!("Universe '{key}' not found — creating");
        if !dry_run {
            let post_url = format!("{base_url}/api/v1/universes");
            let body = serde_json::json!({
                "key": key,
                "name": name,
                "description": "",
            });
            let r = client
                .post(&post_url)
                .bearer_auth(token)
                .json(&body)
                .send()
                .context("POST /universes")?;
            if !r.status().is_success() {
                let status = r.status();
                let text = r.text().unwrap_or_default();
                // CO-423: creating a universe needs a *session* (browser login),
                // not an API token. Surface a clear, actionable message instead
                // of a bare 401 so the operator knows the credential is wrong.
                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    bail!(
                        "cannot create universe '{key}' with an API token (HTTP {status}). \
                         Creating a universe requires a logged-in session. Either create \
                         '{key}' in the web UI first, then re-run `co source add`, or run \
                         against a server where the token's user can create universes."
                    );
                }
                bail!("POST /universes — HTTP {status} — {text}");
            }
        } else {
            eprintln!("  [dry-run] POST {base_url}/api/v1/universes");
        }
    }

    // CO-423: re-parent via the new PUT API (item 1). Owner-only server-side.
    if let Some(parent) = parent {
        if dry_run {
            eprintln!("  [dry-run] PUT {base_url}/api/v1/universes/{key} {{parent_key:{parent}}}");
        } else {
            set_parent(client, base_url, token, key, parent)?;
            eprintln!("Set parent of '{key}' to '{parent}'");
        }
    }
    Ok(())
}

/// CO-423: confirm a universe exists by a REAL signal — parse the universe API
/// JSON and check it has a `key` field matching the requested slug. The SPA
/// returns HTTP 200 (its shell) for unknown routes, so a status check alone
/// false-positives.
fn universe_exists(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    key: &str,
) -> Result<bool> {
    let get_url = format!("{base_url}/api/v1/universes/{key}");
    let resp = client
        .get(&get_url)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .context("GET universe")?;

    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(false);
    }
    if !status.is_success() {
        // 401/403/5xx etc. are real errors, not "exists".
        bail!("GET /universes/{key} — HTTP {status}");
    }
    // 200: only trust it if the body parses as the universe JSON AND the key
    // matches. The SPA shell is HTML, so JSON parse fails → treat as absent.
    let body = resp.text().context("read universe body")?;
    let probe: UniverseInfoProbe = match serde_json::from_str(&body) {
        Ok(p) => p,
        Err(_) => return Ok(false), // HTML shell — not a real universe.
    };
    Ok(probe.key.as_deref() == Some(key))
}

/// CO-423: set `parent_key` via `PUT /api/v1/universes/{key}`.
fn set_parent(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    key: &str,
    parent: &str,
) -> Result<()> {
    let url = format!("{base_url}/api/v1/universes/{key}");
    let body = serde_json::json!({ "parent_key": parent });
    let r = client
        .put(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .context("PUT /universes (parent_key)")?;
    if !r.status().is_success() {
        let status = r.status();
        let text = r.text().unwrap_or_default();
        bail!("PUT /universes/{key} (parent_key) — HTTP {status} — {text}");
    }
    Ok(())
}

fn put_entry(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    universe_key: &str,
    vault_path: &str,
    markdown: &str,
) -> Result<()> {
    let url = format!("{base_url}/api/v1/universes/{universe_key}/vault/{vault_path}");
    // CO-438: back off and retry on 429 instead of aborting the whole import.
    // Admin tokens are exempt server-side, but non-admin bulk pushes can still
    // hit the limit; honour Retry-After (capped) so the import drains rather
    // than failing partway through.
    const MAX_RETRIES: u32 = 5;
    for attempt in 0..=MAX_RETRIES {
        let resp = client
            .put(&url)
            .bearer_auth(token)
            .body(markdown.to_string())
            .send()
            .with_context(|| format!("PUT vault/{vault_path}"))?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < MAX_RETRIES {
            let wait = retry_after_secs(&resp).unwrap_or(1 << attempt).min(30);
            eprintln!("  rate-limited on {vault_path} — retrying in {wait}s");
            std::thread::sleep(std::time::Duration::from_secs(wait));
            continue;
        }
        if !resp.status().is_success() {
            bail!("PUT vault/{vault_path} — HTTP {}", resp.status());
        }
        return Ok(());
    }
    bail!("PUT vault/{vault_path} — still rate-limited after {MAX_RETRIES} retries")
}

/// CO-438: read the `Retry-After` header (seconds) from a 429 response.
fn retry_after_secs(resp: &reqwest::blocking::Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

fn reconcile_deletions(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    universe_key: &str,
    entries: &[MaterializedEntry],
    dry_run: bool,
) -> Result<()> {
    use std::collections::HashSet;
    let imported: HashSet<&str> = entries.iter().map(|e| e.vault_path.as_str()).collect();
    let server_paths = list_vault(client, base_url, token, universe_key)?;
    let mut deleted = 0usize;
    for sp in &server_paths {
        // Only reconcile within the content/ tree we own.
        if !sp.starts_with("content/") {
            continue;
        }
        if !imported.contains(sp.as_str()) {
            if dry_run {
                eprintln!("  delete  {sp}");
            } else {
                delete_entry(client, base_url, token, universe_key, sp)?;
                deleted += 1;
            }
        }
    }
    if !dry_run && deleted > 0 {
        eprintln!("Soft-reflected {deleted} repo deletions");
    }
    Ok(())
}

fn list_vault(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    universe_key: &str,
) -> Result<Vec<String>> {
    let url = format!("{base_url}/api/v1/universes/{universe_key}/vault/");
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .context("GET vault list")?;
    if !resp.status().is_success() {
        bail!("GET vault/ — HTTP {}", resp.status());
    }
    let entries: Vec<VaultEntry> = resp.json().context("parse vault list")?;
    Ok(entries.into_iter().map(|e| e.path).collect())
}

/// CO-438: fetch the server's `path → body_hash` map for a universe so the
/// importer can skip entries whose body is already identical. Entries whose
/// hash the server omits (older server) are left out of the map and therefore
/// always pushed.
fn fetch_server_hashes(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    universe_key: &str,
) -> Result<HashMap<String, String>> {
    let url = format!("{base_url}/api/v1/universes/{universe_key}/vault/");
    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .context("GET vault list (hashes)")?;
    // A brand-new universe (just created above) may legitimately 404 here;
    // treat any non-success as "no known hashes" so every entry is pushed.
    if !resp.status().is_success() {
        return Ok(HashMap::new());
    }
    let entries: Vec<VaultEntry> = resp.json().context("parse vault list (hashes)")?;
    Ok(entries
        .into_iter()
        .filter_map(|e| e.body_hash.map(|h| (e.path, h)))
        .collect())
}

/// CO-438: compute the body hash of a materialized entry the way the server
/// does — strip the YAML frontmatter, then SHA-256 the body. The provenance
/// frontmatter embeds the repo HEAD sha (which advances on every commit), so
/// hashing the sha-independent *body* is what makes "only re-PUT what changed"
/// correct: an unchanged file keeps the same body hash across imports.
fn entry_body_hash(markdown: &str) -> String {
    let body = co::entry::split_frontmatter(markdown)
        .map(|(_, body)| body)
        .unwrap_or_else(|_| markdown.to_string());
    co::entry::Entry::hash_body(&body)
}

/// CO-438: select the entries that actually need pushing. An entry is skipped
/// only when the server reports a byte-identical body hash for its path; a
/// missing path or any difference is pushed. This direction is deliberate — a
/// false "changed" merely re-PUTs (harmless), while a false "unchanged" would
/// silently drop a real update, so we never skip on uncertainty.
fn select_changed<'a>(
    entries: &'a [MaterializedEntry],
    server_hashes: &HashMap<String, String>,
) -> Vec<&'a MaterializedEntry> {
    entries
        .iter()
        .filter(|e| server_hashes.get(&e.vault_path) != Some(&entry_body_hash(&e.markdown)))
        .collect()
}

fn delete_entry(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    universe_key: &str,
    vault_path: &str,
) -> Result<()> {
    let url = format!("{base_url}/api/v1/universes/{universe_key}/vault/{vault_path}");
    let resp = client
        .delete(&url)
        .bearer_auth(token)
        .send()
        .with_context(|| format!("DELETE vault/{vault_path}"))?;
    if !resp.status().is_success() {
        bail!("DELETE vault/{vault_path} — HTTP {}", resp.status());
    }
    Ok(())
}

// ─── small helpers ─────────────────────────────────────────────────────────────

fn short_sha(sha: &str) -> &str {
    if sha.len() >= 7 { &sha[..7] } else { sha }
}

fn sanitize_key(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // ── owner/repo parsing ────────────────────────────────────────────────────

    #[test]
    fn parse_owner_repo_plain() {
        assert_eq!(
            parse_owner_repo("yurisugano/SensorySpeech").unwrap(),
            ("yurisugano".into(), "SensorySpeech".into())
        );
    }

    #[test]
    fn parse_owner_repo_url() {
        assert_eq!(
            parse_owner_repo("https://github.com/foo/bar.git").unwrap(),
            ("foo".into(), "bar".into())
        );
    }

    #[test]
    fn parse_owner_repo_scheme() {
        assert_eq!(
            parse_owner_repo("github:foo/bar").unwrap(),
            ("foo".into(), "bar".into())
        );
    }

    #[test]
    fn parse_owner_repo_rejects_bad() {
        assert!(parse_owner_repo("not-a-repo").is_err());
        assert!(parse_owner_repo("a/b/c").is_err());
    }

    // ── classify / rewrite_ext ─────────────────────────────────────────────────

    #[test]
    fn classify_by_extension() {
        assert_eq!(classify("README.md"), FileKind::Markdown);
        assert_eq!(classify("a/b.MARKDOWN"), FileKind::Markdown);
        assert_eq!(classify("notebooks/demo.ipynb"), FileKind::Notebook);
        assert_eq!(classify("data/x.csv"), FileKind::Asset);
        assert_eq!(classify("Makefile"), FileKind::Asset);
    }

    #[test]
    fn rewrite_ext_preserves_dir() {
        assert_eq!(rewrite_ext("a/b/c.ipynb", "md"), "a/b/c.md");
        assert_eq!(rewrite_ext("top.csv", "md"), "top.md");
    }

    // ── ipynb → markdown (the key transform) ───────────────────────────────────

    #[test]
    fn ipynb_markdown_cell_renders_verbatim() {
        let nb = r##"{
          "cells": [
            {"cell_type": "markdown", "source": ["# Title\n", "\n", "Some text."]}
          ],
          "metadata": {"kernelspec": {"language": "python"}}
        }"##;
        let md = ipynb_to_markdown(nb);
        assert_eq!(md, "# Title\n\nSome text.\n");
    }

    #[test]
    fn ipynb_code_cell_fenced_with_language() {
        let nb = r##"{
          "cells": [
            {"cell_type": "code", "source": ["x = 1\n", "print(x)"]}
          ],
          "metadata": {"kernelspec": {"language": "python"}}
        }"##;
        let md = ipynb_to_markdown(nb);
        assert_eq!(md, "```python\nx = 1\nprint(x)\n```\n");
    }

    #[test]
    fn ipynb_mixed_order_preserved() {
        let nb = r###"{
          "cells": [
            {"cell_type": "markdown", "source": "## Step 1"},
            {"cell_type": "code", "source": "a = 2"},
            {"cell_type": "markdown", "source": "## Step 2"}
          ],
          "metadata": {"language_info": {"name": "python"}}
        }"###;
        let md = ipynb_to_markdown(nb);
        assert_eq!(md, "## Step 1\n\n```python\na = 2\n```\n\n## Step 2\n");
    }

    #[test]
    fn ipynb_empty_cells_skipped() {
        let nb = r##"{
          "cells": [
            {"cell_type": "code", "source": []},
            {"cell_type": "markdown", "source": ["   "]},
            {"cell_type": "code", "source": ["y = 3"]}
          ],
          "metadata": {"kernelspec": {"language": "python"}}
        }"##;
        let md = ipynb_to_markdown(nb);
        assert_eq!(md, "```python\ny = 3\n```\n");
    }

    #[test]
    fn ipynb_language_falls_back_to_python() {
        let nb = r##"{ "cells": [ {"cell_type": "code", "source": "z=1"} ] }"##;
        let md = ipynb_to_markdown(nb);
        assert_eq!(md, "```python\nz=1\n```\n");
    }

    #[test]
    fn ipynb_non_python_language() {
        let nb = r##"{
          "cells": [ {"cell_type": "code", "source": "val x = 1"} ],
          "metadata": {"kernelspec": {"language": "scala"}}
        }"##;
        let md = ipynb_to_markdown(nb);
        assert_eq!(md, "```scala\nval x = 1\n```\n");
    }

    #[test]
    fn ipynb_invalid_json_is_wrapped_not_lost() {
        let md = ipynb_to_markdown("{not json");
        assert!(md.contains("{not json"));
        assert!(md.starts_with("```json"));
    }

    // ── materialize ───────────────────────────────────────────────────────────

    #[test]
    fn materialize_markdown_keeps_body_and_stamps_provenance() {
        let files = vec![SourceFile {
            rel_path: "docs/intro.md".into(),
            text: Some("# Intro\n\nHello".into()),
        }];
        let out = materialize(&files, "foo/bar", "abc123");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].vault_path, "content/docs/intro.md");
        assert!(out[0].markdown.contains("source: github:foo/bar@abc123"));
        assert!(out[0].markdown.contains("source_path: docs/intro.md"));
        assert!(out[0].markdown.contains("# Intro\n\nHello"));
    }

    #[test]
    fn materialize_ipynb_becomes_markdown_entry() {
        let files = vec![SourceFile {
            rel_path: "nb/demo.ipynb".into(),
            text: Some(
                r##"{"cells":[{"cell_type":"code","source":"print(1)"}],
                   "metadata":{"kernelspec":{"language":"python"}}}"##
                    .into(),
            ),
        }];
        let out = materialize(&files, "foo/bar", "deadbeef");
        assert_eq!(out[0].vault_path, "content/nb/demo.md");
        assert!(out[0].markdown.contains("```python\nprint(1)\n```"));
        assert!(out[0].markdown.contains("source_path: nb/demo.ipynb"));
        assert!(out[0].markdown.contains("entry_type: notebook"));
    }

    #[test]
    fn materialize_asset_links_original() {
        let files = vec![SourceFile {
            rel_path: "data/sample.wav".into(),
            text: None,
        }];
        let out = materialize(&files, "foo/bar", "sha9");
        assert_eq!(out[0].vault_path, "content/data/sample.md");
        assert!(out[0].markdown.contains("entry_type: asset"));
        assert!(
            out[0]
                .markdown
                .contains("raw.githubusercontent.com/foo/bar/sha9/data/sample.wav")
        );
    }

    #[test]
    fn materialize_is_deterministic_idempotent() {
        let files = vec![SourceFile {
            rel_path: "a.md".into(),
            text: Some("x".into()),
        }];
        let a = materialize(&files, "o/r", "s");
        let b = materialize(&files, "o/r", "s");
        assert_eq!(a, b);
    }

    // ── collect_from_dir over a fixture tree ───────────────────────────────────

    #[test]
    fn collect_preserves_hierarchy_and_skips_dotfiles() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("notebooks")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("README.md"), "# readme").unwrap();
        fs::write(root.join("notebooks/demo.ipynb"), "{}").unwrap();
        fs::write(root.join(".git/config"), "junk").unwrap();
        fs::write(root.join(".hidden"), "junk").unwrap();

        let files = collect_from_dir(root).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.rel_path.as_str()).collect();
        assert_eq!(paths, vec!["README.md", "notebooks/demo.ipynb"]);
    }

    /// End-to-end (no network): a fixture repo with a `.md` and a `.ipynb`
    /// produces the expected hierarchical tree and a readable ipynb→md entry.
    #[test]
    fn fixture_repo_produces_expected_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("analysis")).unwrap();
        fs::write(root.join("README.md"), "# Project\n\nIntro.").unwrap();
        fs::write(
            root.join("analysis/run.ipynb"),
            r###"{
              "cells": [
                {"cell_type": "markdown", "source": "## Analysis"},
                {"cell_type": "code", "source": ["import numpy as np\n", "np.mean([1,2,3])"]}
              ],
              "metadata": {"kernelspec": {"language": "python"}}
            }"###,
        )
        .unwrap();

        let files = collect_from_dir(root).unwrap();
        let entries = materialize(&files, "yuri/nlp", "cafe1234");

        let vault_paths: Vec<&str> = entries.iter().map(|e| e.vault_path.as_str()).collect();
        assert_eq!(
            vault_paths,
            vec!["content/README.md", "content/analysis/run.md"]
        );

        let nb_entry = &entries[1];
        assert!(nb_entry.markdown.contains("## Analysis"));
        assert!(
            nb_entry
                .markdown
                .contains("```python\nimport numpy as np\nnp.mean([1,2,3])\n```")
        );
        assert!(
            nb_entry
                .markdown
                .contains("source: github:yuri/nlp@cafe1234")
        );
        assert!(
            nb_entry
                .markdown
                .contains("source_path: analysis/run.ipynb")
        );
    }

    #[test]
    fn sanitize_key_works() {
        assert_eq!(sanitize_key("SensorySpeech"), "sensoryspeech");
        assert_eq!(sanitize_key("My_Repo.v2"), "my-repo-v2");
    }

    // ── auto-index / landing (CO-423) ──────────────────────────────────────────

    #[test]
    fn has_index_entry_detects_content_root_index() {
        let with = vec![MaterializedEntry {
            vault_path: "content/index.md".into(),
            markdown: "x".into(),
        }];
        assert!(has_index_entry(&with));

        // case-insensitive
        let caps = vec![MaterializedEntry {
            vault_path: "content/INDEX.md".into(),
            markdown: "x".into(),
        }];
        assert!(has_index_entry(&caps));

        // a nested index is NOT the content-root landing
        let nested = vec![MaterializedEntry {
            vault_path: "content/docs/index.md".into(),
            markdown: "x".into(),
        }];
        assert!(!has_index_entry(&nested));
    }

    #[test]
    fn synth_index_links_imported_tree() {
        let entries = vec![
            MaterializedEntry {
                vault_path: "content/README.md".into(),
                markdown: "x".into(),
            },
            MaterializedEntry {
                vault_path: "content/analysis/run.md".into(),
                markdown: "y".into(),
            },
        ];
        let idx = synth_index(&entries, "yuri/nlp", "nlp");
        assert_eq!(idx.vault_path, "content/index.md");
        assert!(idx.markdown.contains("entry_type: page"));
        assert!(idx.markdown.contains("[README](README.md)"));
        assert!(idx.markdown.contains("[run](analysis/run.md)"));
        assert!(idx.markdown.contains("github.com/yuri/nlp"));
    }

    /// The materialize+index pipeline synthesizes a landing only when the repo
    /// brought none, and never overwrites an existing `index.md`.
    #[test]
    fn index_is_synthesized_only_when_absent() {
        // No index.md in the repo → one is synthesized at the front.
        let files = vec![SourceFile {
            rel_path: "README.md".into(),
            text: Some("# Readme".into()),
        }];
        let mut entries = materialize(&files, "o/r", "sha");
        assert!(!has_index_entry(&entries));
        entries.insert(0, synth_index(&entries, "o/r", "r"));
        assert_eq!(entries[0].vault_path, "content/index.md");
        assert_eq!(has_index_entry(&entries) as usize, 1);

        // Repo already ships index.md → has_index_entry short-circuits, no synth.
        let files = vec![SourceFile {
            rel_path: "index.md".into(),
            text: Some("# Home".into()),
        }];
        let entries = materialize(&files, "o/r", "sha");
        assert!(has_index_entry(&entries));
        assert!(entries[0].markdown.contains("# Home"));
    }

    // ── CO-438: skip-if-identical (resume) ──────────────────────────────────

    /// entry_body_hash matches the server's `body_hash` (hash of the body with
    /// frontmatter stripped), so an unchanged entry hashes identically.
    #[test]
    fn entry_body_hash_strips_frontmatter() {
        let md = "---\ntitle: T\nsource: github:o/r@abc\n---\n\n# Body\n\ntext";
        // Equivalent to the server: split_frontmatter then hash_body.
        let expected = co::entry::Entry::hash_body("# Body\n\ntext");
        assert_eq!(entry_body_hash(md), expected);
    }

    /// The provenance frontmatter carries the repo HEAD sha, which advances on
    /// every commit. The body hash must ignore it so an unchanged file is still
    /// recognised as unchanged after the sha bumps.
    #[test]
    fn entry_body_hash_ignores_provenance_sha() {
        let files = vec![SourceFile {
            rel_path: "docs/a.md".into(),
            text: Some("# A\n\nsame body".into()),
        }];
        let v1 = materialize(&files, "o/r", "sha_one");
        let v2 = materialize(&files, "o/r", "sha_two");
        assert_ne!(v1[0].markdown, v2[0].markdown, "frontmatter sha differs");
        assert_eq!(
            entry_body_hash(&v1[0].markdown),
            entry_body_hash(&v2[0].markdown),
            "body hash must be sha-independent"
        );
    }

    #[test]
    fn select_changed_skips_identical_and_pushes_new_or_modified() {
        let files = vec![
            SourceFile {
                rel_path: "a.md".into(),
                text: Some("# A\n\nunchanged".into()),
            },
            SourceFile {
                rel_path: "b.md".into(),
                text: Some("# B\n\nmodified".into()),
            },
            SourceFile {
                rel_path: "c.md".into(),
                text: Some("# C\n\nbrand new".into()),
            },
        ];
        let entries = materialize(&files, "o/r", "sha");

        // Server has `a` identical, `b` with an old (different) body, no `c`.
        let mut server = HashMap::new();
        server.insert(
            "content/a.md".to_string(),
            entry_body_hash(&entries[0].markdown),
        );
        server.insert(
            "content/b.md".to_string(),
            co::entry::Entry::hash_body("# B\n\nOLD body"),
        );

        let to_push = select_changed(&entries, &server);
        let paths: Vec<&str> = to_push.iter().map(|e| e.vault_path.as_str()).collect();
        assert_eq!(paths, vec!["content/b.md", "content/c.md"]);
    }

    /// Re-syncing an unchanged repo pushes nothing.
    #[test]
    fn select_changed_resync_unchanged_pushes_nothing() {
        let files = vec![
            SourceFile {
                rel_path: "a.md".into(),
                text: Some("# A".into()),
            },
            SourceFile {
                rel_path: "b.md".into(),
                text: Some("# B".into()),
            },
        ];
        let entries = materialize(&files, "o/r", "sha");
        let server: HashMap<String, String> = entries
            .iter()
            .map(|e| (e.vault_path.clone(), entry_body_hash(&e.markdown)))
            .collect();
        assert!(select_changed(&entries, &server).is_empty());
    }

    /// An entry the server lacks a hash for (older server) is always pushed —
    /// never wrongly skipped.
    #[test]
    fn select_changed_pushes_when_server_hash_unknown() {
        let files = vec![SourceFile {
            rel_path: "a.md".into(),
            text: Some("# A".into()),
        }];
        let entries = materialize(&files, "o/r", "sha");
        let server = HashMap::new();
        assert_eq!(select_changed(&entries, &server).len(), 1);
    }
}
