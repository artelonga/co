//! `co push` — upload a local universe to a remote CO server over the Vault API.
//!
//! Wraps `POST /api/v1/universes` + Vault `PUT`s so the CLI and web front-end
//! hit identical endpoints. Re-running converges (idempotent).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use colored::Colorize;
use serde::{Deserialize, Serialize};

use crate::commands::auth;

const DEFAULT_BASE_URL: &str = "https://co.artelonga.com.br";

// ─── Request/response types ───────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct CreateUniverseReq {
    key: String,
    name: String,
    description: String,
}

#[derive(Debug, Serialize)]
struct UpdateUniverseReq {
    name: String,
}

#[derive(Debug, Deserialize)]
struct VaultEntry {
    path: String,
}

// ─── Entry point ──────────────────────────────────────────────────────────────

pub fn run(
    remote: Option<String>,
    token: Option<String>,
    key: Option<String>,
    dry_run: bool,
    delete_missing: bool,
) {
    if let Err(e) = do_push(remote, token, key, dry_run, delete_missing) {
        eprintln!("{} {:#}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}

fn do_push(
    remote: Option<String>,
    token: Option<String>,
    key: Option<String>,
    dry_run: bool,
    delete_missing: bool,
) -> Result<()> {
    let (base_url, api_token) = resolve_auth(remote, token)?;

    let cwd = std::env::current_dir().context("cannot read CWD")?;
    let repo_root = find_repo_root(&cwd);

    let basename = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("universe")
        .to_string();

    let universe_key = key.unwrap_or_else(|| sanitize_key(&basename));
    let display_name = basename;

    let content_dir = repo_root.join("content");
    if !content_dir.exists() {
        bail!("no content/ directory at {}", repo_root.display());
    }

    let gitignore_patterns = read_gitignore(&repo_root);
    let local_files = collect_files(&content_dir, &repo_root, &gitignore_patterns)?;

    if dry_run {
        eprintln!("{}", "dry-run — no writes will be made".yellow().bold());
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent("co-cli/push")
        .build()
        .context("build HTTP client")?;

    // 1. Ensure universe exists (create or update).
    ensure_universe(
        &client,
        &base_url,
        &api_token,
        &universe_key,
        &display_name,
        dry_run,
    )?;

    // 2. Push entries via Vault PUT.
    let total = local_files.len();
    eprintln!("Pushing {} entries...", total);
    for (abs_path, vault_path) in &local_files {
        if dry_run {
            eprintln!("  update  {}", vault_path);
        } else {
            push_entry(
                &client,
                &base_url,
                &api_token,
                &universe_key,
                abs_path,
                vault_path,
            )?;
        }
    }

    // 3. Optional reconcile: delete server entries absent locally.
    if delete_missing {
        let server_paths = list_vault(&client, &base_url, &api_token, &universe_key)?;
        let local_set: HashSet<&str> = local_files.iter().map(|(_, v)| v.as_str()).collect();
        let mut deleted = 0usize;
        for sp in &server_paths {
            if !local_set.contains(sp.as_str()) {
                if dry_run {
                    eprintln!("  delete  {}", sp);
                } else {
                    delete_entry(&client, &base_url, &api_token, &universe_key, sp)?;
                    deleted += 1;
                }
            }
        }
        if !dry_run && deleted > 0 {
            eprintln!("Deleted {} remote entries", deleted);
        }
    }

    if dry_run {
        eprintln!("dry-run complete — {} entries would be pushed", total);
    } else {
        eprintln!(
            "{} '{}' — {} entries",
            "pushed".green().bold(),
            universe_key,
            total
        );
    }

    Ok(())
}

// ─── Auth resolution ─────────────────────────────────────────────────────────

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

// ─── Universe management ──────────────────────────────────────────────────────

fn ensure_universe(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    key: &str,
    name: &str,
    dry_run: bool,
) -> Result<()> {
    let get_url = format!("{}/api/v1/universes/{}", base_url, key);
    let resp = client
        .get(&get_url)
        .bearer_auth(token)
        .send()
        .context("GET universe")?;

    if resp.status().is_success() {
        eprintln!("Universe '{}' exists — updating", key);
        if !dry_run {
            let body = UpdateUniverseReq {
                name: name.to_string(),
            };
            let r = client
                .put(&get_url)
                .bearer_auth(token)
                .json(&body)
                .send()
                .context("PUT universe")?;
            if !r.status().is_success() {
                bail!("PUT /universes/{} — HTTP {}", key, r.status());
            }
        } else {
            eprintln!("  [dry-run] PUT {}", get_url);
        }
    } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
        eprintln!("Universe '{}' not found — creating", key);
        if !dry_run {
            let post_url = format!("{}/api/v1/universes", base_url);
            let body = CreateUniverseReq {
                key: key.to_string(),
                name: name.to_string(),
                description: String::new(),
            };
            let r = client
                .post(&post_url)
                .bearer_auth(token)
                .json(&body)
                .send()
                .context("POST /universes")?;
            if !r.status().is_success() {
                let status = r.status();
                let text = r.text().unwrap_or_default();
                bail!("POST /universes — HTTP {} — {}", status, text);
            }
        } else {
            eprintln!("  [dry-run] POST {}/api/v1/universes", base_url);
        }
    } else {
        bail!("GET /universes/{} — HTTP {}", key, resp.status());
    }

    Ok(())
}

// ─── Vault operations ─────────────────────────────────────────────────────────

fn push_entry(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    universe_key: &str,
    abs_path: &Path,
    vault_path: &str,
) -> Result<()> {
    let content = std::fs::read_to_string(abs_path)
        .with_context(|| format!("reading {}", abs_path.display()))?;
    let url = format!(
        "{}/api/v1/universes/{}/vault/{}",
        base_url, universe_key, vault_path
    );
    let resp = client
        .put(&url)
        .bearer_auth(token)
        .body(content)
        .send()
        .with_context(|| format!("PUT vault/{}", vault_path))?;
    if !resp.status().is_success() {
        bail!("PUT vault/{} — HTTP {}", vault_path, resp.status());
    }
    Ok(())
}

fn list_vault(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    universe_key: &str,
) -> Result<Vec<String>> {
    let url = format!("{}/api/v1/universes/{}/vault/", base_url, universe_key);
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

fn delete_entry(
    client: &reqwest::blocking::Client,
    base_url: &str,
    token: &str,
    universe_key: &str,
    vault_path: &str,
) -> Result<()> {
    let url = format!(
        "{}/api/v1/universes/{}/vault/{}",
        base_url, universe_key, vault_path
    );
    let resp = client
        .delete(&url)
        .bearer_auth(token)
        .send()
        .with_context(|| format!("DELETE vault/{}", vault_path))?;
    if !resp.status().is_success() {
        bail!("DELETE vault/{} — HTTP {}", vault_path, resp.status());
    }
    Ok(())
}

// ─── File collection ──────────────────────────────────────────────────────────

/// Returns `(absolute_path, vault_path)` for all pushable `.md` files under
/// `content_dir`. `vault_path` is relative to `repo_root`
/// (e.g. `content/projects/CO/1.md`), matching the path the server stores.
fn collect_files(
    content_dir: &Path,
    repo_root: &Path,
    gitignore_patterns: &[String],
) -> Result<Vec<(PathBuf, String)>> {
    let mut files = Vec::new();
    collect_recursive(content_dir, repo_root, gitignore_patterns, &mut files)?;
    files.sort_by(|(_, a), (_, b)| a.cmp(b));
    Ok(files)
}

fn collect_recursive(
    dir: &Path,
    repo_root: &Path,
    gitignore_patterns: &[String],
    out: &mut Vec<(PathBuf, String)>,
) -> Result<()> {
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

        // Skip hidden files/directories.
        if name.starts_with('.') {
            continue;
        }

        // Skip _source/ (PII originals — LGPD policy).
        if name == "_source" {
            continue;
        }

        // Compute path relative to repo_root for gitignore checks and vault addressing.
        let rel = path
            .strip_prefix(repo_root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        if is_gitignored(&rel, gitignore_patterns) {
            continue;
        }

        if path.is_dir() {
            collect_recursive(&path, repo_root, gitignore_patterns, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push((path, rel));
        }
    }

    Ok(())
}

// ─── .gitignore ───────────────────────────────────────────────────────────────

fn read_gitignore(repo_root: &Path) -> Vec<String> {
    std::fs::read_to_string(repo_root.join(".gitignore"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
        .collect()
}

fn is_gitignored(rel_path: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        // Strip trailing slash (directory patterns).
        let p = pattern.trim_end_matches('/');
        // Exact path match.
        if rel_path == p {
            return true;
        }
        // Any path component matches (e.g. "node_modules" matches anywhere).
        if rel_path.split('/').any(|c| c == p) {
            return true;
        }
        // "*.ext" wildcard: matches any file with that extension.
        if let Some(ext) = p.strip_prefix("*.")
            && rel_path.ends_with(&format!(".{}", ext))
        {
            return true;
        }
        // Prefix: "foo/" pattern matches "foo/bar" via the p-without-trailing-slash form.
        if rel_path.starts_with(&format!("{}/", p)) {
            return true;
        }
    }
    false
}

// ─── Repo root discovery ─────────────────────────────────────────────────────

fn find_repo_root(start: &Path) -> PathBuf {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() || dir.join(".jj").exists() {
            return dir;
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return start.to_path_buf(),
        }
    }
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

    // ── sanitize_key ─────────────────────────────────────────────────────────

    #[test]
    fn sanitize_key_lowercases() {
        assert_eq!(sanitize_key("MyProject"), "myproject");
    }

    #[test]
    fn sanitize_key_replaces_spaces() {
        assert_eq!(sanitize_key("my project"), "my-project");
    }

    #[test]
    fn sanitize_key_collapses_dashes() {
        assert_eq!(sanitize_key("my--project"), "my-project");
    }

    #[test]
    fn sanitize_key_strips_leading_trailing() {
        assert_eq!(sanitize_key("_project_"), "project");
    }

    // ── is_gitignored ─────────────────────────────────────────────────────────

    #[test]
    fn gitignore_wildcard_ext() {
        assert!(is_gitignored("content/foo.log", &["*.log".into()]));
    }

    #[test]
    fn gitignore_component_match() {
        assert!(is_gitignored(
            "content/node_modules/foo.md",
            &["node_modules".into()]
        ));
    }

    #[test]
    fn gitignore_directory_pattern() {
        assert!(is_gitignored("content/drafts/foo.md", &["drafts/".into()]));
    }

    #[test]
    fn gitignore_no_match() {
        assert!(!is_gitignored("content/foo.md", &["*.log".into()]));
    }

    // ── collect_files ─────────────────────────────────────────────────────────

    #[test]
    fn collect_skips_source_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let content = root.join("content");
        fs::create_dir(&content).unwrap();

        let source = content.join("_source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("secret.md"), "pii").unwrap();
        fs::write(content.join("visible.md"), "# visible").unwrap();

        let files = collect_files(&content, root, &[]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, "content/visible.md");
    }

    #[test]
    fn collect_skips_gitignored() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let content = root.join("content");
        fs::create_dir(&content).unwrap();
        fs::write(content.join("page.md"), "# page").unwrap();
        fs::write(content.join("draft.md"), "# draft").unwrap();

        let files = collect_files(&content, root, &["draft.md".into()]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, "content/page.md");
    }

    #[test]
    fn collect_skips_hidden() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let content = root.join("content");
        fs::create_dir(&content).unwrap();
        fs::write(content.join(".hidden.md"), "# hidden").unwrap();
        fs::write(content.join("visible.md"), "# visible").unwrap();

        let files = collect_files(&content, root, &[]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, "content/visible.md");
    }

    #[test]
    fn collect_vault_path_is_relative_to_repo_root() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let content = root.join("content");
        let sub = content.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("page.md"), "# page").unwrap();

        let files = collect_files(&content, root, &[]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, "content/sub/page.md");
    }

    // ── find_repo_root ────────────────────────────────────────────────────────

    #[test]
    fn find_repo_root_finds_git() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("content");
        fs::create_dir(&sub).unwrap();
        assert_eq!(find_repo_root(&sub), root);
    }

    #[test]
    fn find_repo_root_finds_jj() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir(root.join(".jj")).unwrap();
        let sub = root.join("content");
        fs::create_dir(&sub).unwrap();
        assert_eq!(find_repo_root(&sub), root);
    }

    #[test]
    fn find_repo_root_fallback() {
        let tmp = tempdir().unwrap();
        assert_eq!(find_repo_root(tmp.path()), tmp.path());
    }
}
