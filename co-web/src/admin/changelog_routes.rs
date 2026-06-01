//! GET /api/v1/changelog — cross-version changelog viewer (CO-260)
//! GET /api/v1/changelog/feed — multi-repo interleaved feed (CO-334)
//! GET /api/v1/changelog/repos — list known repos (CO-334)
//! POST /api/v1/admin/changelog/reindex — rebuild cache from git log

use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::server::AppState;
use crate::storage::changelog::ChangelogCacheRow;
use crate::storage::release_notes::ReleaseNoteRow;

use super::changelog_parser::parse_keep_a_changelog;

// ---------------------------------------------------------------------------
// Embedded assets
// ---------------------------------------------------------------------------

const CHANGELOG_MD: &str = include_str!("../../../CHANGELOG.md");
const CHANGELOG_PAGE_HTML: &str = include_str!("../../static/variants/a/changelog.html");

// " — " separator used throughout CHANGELOG.md (space + U+2014 em-dash + space)
const SEP: &str = " \u{2014} ";

// ---------------------------------------------------------------------------
// Routers
// ---------------------------------------------------------------------------

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/changelog", get(get_changelog))
        // CO-334: multi-repo interleaved feed
        .route("/changelog/feed", get(get_changelog_feed))
        .route("/changelog/repos", get(get_changelog_repos))
}

pub fn admin_router() -> Router<AppState> {
    Router::new().route("/changelog/reindex", post(reindex_changelog))
}

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ChangelogQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    /// "size" sorts entries by pr_size desc; "date" (default) keeps newest-first order.
    pub sort: Option<String>,
}

#[derive(Serialize)]
pub struct ChangelogResponse {
    pub range: RangeInfo,
    pub versions: Vec<VersionBlock>,
}

#[derive(Serialize)]
pub struct RangeInfo {
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Serialize)]
pub struct VersionBlock {
    pub version: String,
    pub date: String,
    pub tag_sha: Option<String>,
    pub theme: String,
    pub entries: Vec<ChangelogEntry>,
}

#[derive(Serialize, Clone)]
pub struct ChangelogEntry {
    pub ticket: String,
    pub entry_type: String,
    pub title: String,
    pub pr: Option<i64>,
    pub pr_size: Option<i64>,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub commit_sha: Option<String>,
    pub author: Option<String>,
    pub spec_url: String,
}

// ---------------------------------------------------------------------------
// Semver helpers
// ---------------------------------------------------------------------------

fn parse_semver(v: &str) -> Option<(u32, u32, u32)> {
    let s = v.trim_start_matches('v');
    let mut it = s.splitn(3, '.');
    Some((
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    ))
}

fn semver_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (parse_semver(a), parse_semver(b)) {
        (Some(av), Some(bv)) => av.cmp(&bv),
        _ => a.cmp(b),
    }
}

// ---------------------------------------------------------------------------
// CHANGELOG.md parser
// ---------------------------------------------------------------------------

fn parse_changelog(content: &str) -> Vec<VersionBlock> {
    let mut versions: Vec<VersionBlock> = Vec::new();
    let mut current: Option<VersionBlock> = None;

    for line in content.lines() {
        // Version header: ## [2.22.0] — 2026-05-21 — Wave C — theme
        if let Some(rest) = line.strip_prefix("## [")
            && let Some(bracket_end) = rest.find(']')
        {
            let version = rest[..bracket_end].to_string();
            // everything after "]" looks like " — 2026-05-21 — theme..."
            let after = &rest[bracket_end + 1..];
            let mut parts = after.splitn(3, SEP);
            let _leading = parts.next();
            let date = parts.next().unwrap_or("").trim().to_string();
            let theme = parts.next().unwrap_or("").trim().to_string();

            if let Some(prev) = current.take() {
                versions.push(prev);
            }
            current = Some(VersionBlock {
                version,
                date,
                tag_sha: None,
                theme,
                entries: Vec::new(),
            });
            continue;
        }

        // Entry header: ## CO-134 — title
        if let Some(rest) = line.strip_prefix("## CO-")
            && let Some(ver) = current.as_mut()
            && let Some(sep_pos) = rest.find(SEP)
            && rest[..sep_pos].chars().all(|c| c.is_ascii_digit())
        {
            let id_part = &rest[..sep_pos];
            let title = rest[sep_pos + SEP.len()..].to_string();
            ver.entries.push(ChangelogEntry {
                ticket: format!("CO-{id_part}"),
                entry_type: "feat".to_string(),
                title,
                pr: None,
                pr_size: None,
                additions: None,
                deletions: None,
                commit_sha: None,
                author: None,
                spec_url: format!("/co/CO-{id_part}"),
            });
        }
    }

    if let Some(v) = current {
        versions.push(v);
    }

    versions
}

// ---------------------------------------------------------------------------
// Cache enrichment
// ---------------------------------------------------------------------------

fn enrich_from_cache(versions: &mut [VersionBlock], cache: &[ChangelogCacheRow]) {
    let lookup: HashMap<(&str, &str), &ChangelogCacheRow> = cache
        .iter()
        .map(|r| ((r.version.as_str(), r.ticket.as_str()), r))
        .collect();

    for ver in versions.iter_mut() {
        for entry in ver.entries.iter_mut() {
            let key = (ver.version.as_str(), entry.ticket.as_str());
            if let Some(row) = lookup.get(&key) {
                entry.entry_type.clone_from(&row.entry_type);
                if !row.title.is_empty() {
                    entry.title.clone_from(&row.title);
                }
                entry.pr = row.pr_number;
                entry.pr_size = row.pr_size;
                entry.additions = row.additions;
                entry.deletions = row.deletions;
                entry.commit_sha.clone_from(&row.commit_sha);
                entry.author.clone_from(&row.author);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Git log scanner (best-effort; returns empty Vec when git is unavailable)
// ---------------------------------------------------------------------------

/// Extract (commit_type, ticket, pr_number) from a conventional commit subject.
/// e.g. `feat(integrations): CO-243 — VS Code extension (#75)`
fn parse_commit_subject(subject: &str) -> Option<(String, String, Option<i64>)> {
    let type_end = subject.find(['(', ':'])?;
    let commit_type = subject[..type_end].to_string();
    if commit_type.is_empty() || commit_type.contains(' ') {
        return None;
    }

    let ticket_start = subject.find("CO-")?;
    let ticket_rest = &subject[ticket_start + 3..];
    let ticket_end = ticket_rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(ticket_rest.len());
    if ticket_end == 0 {
        return None;
    }
    let ticket = format!("CO-{}", &ticket_rest[..ticket_end]);

    let pr = subject.rfind("(#").and_then(|pos| {
        let rest = &subject[pos + 2..];
        let end = rest.find(')')?;
        rest[..end].parse::<i64>().ok()
    });

    Some((commit_type, ticket, pr))
}

pub fn scan_git_log() -> Vec<ChangelogCacheRow> {
    let tags_out = std::process::Command::new("git")
        .args(["tag", "-l", "v*", "--sort=-version:refname"])
        .output();

    let version_tags: Vec<String> = tags_out
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    let t = l.trim();
                    if t.starts_with('v') {
                        Some(t.trim_start_matches('v').to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let log_out = std::process::Command::new("git")
        .args(["log", "--pretty=format:%D\t%s", "--decorate=full"])
        .output();

    let log_text = match log_out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return vec![],
    };

    let mut current_version: Option<String> = None;
    let mut rows: Vec<ChangelogCacheRow> = Vec::new();

    for raw_line in log_text.lines() {
        let mut parts = raw_line.splitn(2, '\t');
        let (refs, subject) = match (parts.next(), parts.next()) {
            (Some(r), Some(s)) => (r, s),
            _ => continue,
        };

        // Check for a version tag on this commit
        for tag in refs.split(',') {
            let tag = tag.trim();
            let tag_name = tag
                .trim_start_matches("tag: refs/tags/")
                .trim_start_matches("tag: ");
            if tag_name.starts_with('v') {
                let ver = tag_name.trim_start_matches('v').to_string();
                if version_tags.contains(&ver) {
                    current_version = Some(ver);
                }
            }
        }

        if let Some(ref ver) = current_version
            && let Some((commit_type, ticket, pr)) = parse_commit_subject(subject)
        {
            rows.push(ChangelogCacheRow {
                version: ver.clone(),
                ticket,
                entry_type: commit_type,
                title: String::new(),
                pr_number: pr,
                pr_size: None,
                additions: None,
                deletions: None,
                commit_sha: None,
                author: None,
            });
        }
    }

    rows
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn serve_changelog_page() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        CHANGELOG_PAGE_HTML,
    )
        .into_response()
}

pub async fn get_changelog(
    State(state): State<AppState>,
    Query(params): Query<ChangelogQuery>,
) -> Result<Json<ChangelogResponse>, AppError> {
    let mut versions = parse_changelog(CHANGELOG_MD);

    {
        let storage = state.core.storage.lock();
        let cache = storage.query_changelog_cache_all();
        enrich_from_cache(&mut versions, &cache);
    }

    // Filter by version range
    if params.from.is_some() || params.to.is_some() {
        versions.retain(|v| {
            if let Some(ref from) = params.from
                && semver_cmp(&v.version, from) == std::cmp::Ordering::Less
            {
                return false;
            }
            if let Some(ref to) = params.to
                && semver_cmp(&v.version, to) == std::cmp::Ordering::Greater
            {
                return false;
            }
            true
        });
    }

    // Filter entries by type
    if let Some(ref type_filter) = params.entry_type {
        for ver in &mut versions {
            ver.entries.retain(|e| &e.entry_type == type_filter);
        }
        versions.retain(|v| !v.entries.is_empty());
    }

    // Drop non-semver entries like [Unreleased] — they have no numeric version
    // to sort against and would sort arbitrarily (string compare vs semver).
    versions.retain(|v| parse_semver(&v.version).is_some());

    // Always sort versions newest-first
    versions.sort_by(|a, b| semver_cmp(&b.version, &a.version));

    // Sort entries within each version
    if params.sort.as_deref() == Some("size") {
        for ver in &mut versions {
            ver.entries
                .sort_by(|a, b| b.pr_size.unwrap_or(0).cmp(&a.pr_size.unwrap_or(0)));
        }
    }

    Ok(Json(ChangelogResponse {
        range: RangeInfo {
            from: params.from,
            to: params.to,
        },
        versions,
    }))
}

pub async fn reindex_changelog(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = scan_git_log();
    let count = rows.len();
    {
        let storage = state.core.storage.lock();
        storage.upsert_changelog_cache_batch(&rows)?;
    }
    Ok(Json(serde_json::json!({ "indexed": count })))
}

// ---------------------------------------------------------------------------
// CO-334 — Feed request/response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct FeedQuery {
    pub repo: Option<String>,
    pub since: Option<String>,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
}

fn default_page() -> u32 {
    1
}
fn default_per_page() -> u32 {
    50
}

#[derive(Serialize)]
pub struct FeedItem {
    pub repo: String,
    pub version: String,
    pub date: String,
    pub theme: Option<String>,
    pub body_md: String,
    pub body_html: String,
}

#[derive(Serialize)]
pub struct FeedResponse {
    pub page: u32,
    pub per_page: u32,
    pub items: Vec<FeedItem>,
}

#[derive(Serialize)]
pub struct RepoInfo {
    pub repo: String,
    pub latest_version: String,
    pub latest_date: String,
}

// ---------------------------------------------------------------------------
// CO-334 — Minimal markdown → HTML renderer for changelog bodies
// ---------------------------------------------------------------------------

fn render_body_html(md: &str) -> String {
    let mut html = String::with_capacity(md.len() + 256);
    let mut in_list = false;

    for line in md.lines() {
        let t = line.trim();
        if t.is_empty() {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            continue;
        }
        if let Some(h) = t.strip_prefix("### ") {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            html.push_str("<h3>");
            html.push_str(&esc_html(h));
            html.push_str("</h3>");
        } else if let Some(h) = t.strip_prefix("## ") {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            html.push_str("<h2>");
            html.push_str(&esc_html(h));
            html.push_str("</h2>");
        } else if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            if !in_list {
                html.push_str("<ul>");
                in_list = true;
            }
            html.push_str("<li>");
            html.push_str(&esc_html(item));
            html.push_str("</li>");
        } else {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            html.push_str("<p>");
            html.push_str(&esc_html(t));
            html.push_str("</p>");
        }
    }
    if in_list {
        html.push_str("</ul>");
    }
    html
}

fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn row_to_feed_item(row: ReleaseNoteRow) -> FeedItem {
    let body_html = render_body_html(&row.body_md);
    FeedItem {
        repo: row.repo,
        version: row.version,
        date: row.date,
        theme: row.theme,
        body_md: row.body_md,
        body_html,
    }
}

// ---------------------------------------------------------------------------
// CO-334 — Release notes refresh orchestrator
// ---------------------------------------------------------------------------

/// Expands `~/` at the start of a path using the home directory.
fn expand_home(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    std::path::PathBuf::from(path)
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parses `notes` into `ReleaseNoteRow`s ready for DB insertion.
fn notes_to_rows(
    notes: Vec<crate::changelog_parser::ReleaseNote>,
    now: i64,
) -> Vec<ReleaseNoteRow> {
    notes
        .into_iter()
        .map(|n| ReleaseNoteRow {
            repo: n.repo,
            version: n.version,
            date: n.date.to_string(),
            theme: n.theme,
            body_md: n.body_md,
            body_text: n.body_text,
            ingested_at: now,
        })
        .collect()
}

/// Reads all configured sister-repo CHANGELOG.md files + CO's own embedded
/// changelog, parses them, and upserts into `release_notes`. Idempotent.
pub async fn run_release_notes_refresh(state: &AppState) -> anyhow::Result<()> {
    let now = unix_now();

    // 1. Ingest CO's own embedded CHANGELOG (no filesystem read needed).
    {
        let rows = notes_to_rows(parse_keep_a_changelog("co", CHANGELOG_MD), now);
        let storage = state.core.storage.lock();
        if let Err(e) = storage.upsert_release_notes_batch(&rows) {
            tracing::warn!("release_notes: CO batch upsert failed: {e}");
        }
    }

    // 2. Get all universe→repo path bindings (brief lock, then release).
    let repo_paths = {
        let storage = state.core.storage.lock();
        storage.query_universe_repo_paths()
    };

    // 3. For each repo, read CHANGELOG.md from the filesystem and ingest.
    for (universe_key, local_repo_path) in repo_paths {
        let changelog_path = expand_home(&local_repo_path).join("CHANGELOG.md");
        let content = match std::fs::read_to_string(&changelog_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(
                    "release_notes: {universe_key}: no CHANGELOG.md at {}: {e}",
                    changelog_path.display()
                );
                continue;
            }
        };

        let notes = parse_keep_a_changelog(&universe_key, &content);
        if notes.is_empty() {
            continue;
        }

        let rows = notes_to_rows(notes, now);
        let storage = state.core.storage.lock();
        if let Err(e) = storage.upsert_release_notes_batch(&rows) {
            tracing::warn!("release_notes: {universe_key} batch upsert failed: {e}");
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// CO-334 — Feed handlers
// ---------------------------------------------------------------------------

pub async fn get_changelog_feed(
    State(state): State<AppState>,
    Query(params): Query<FeedQuery>,
) -> Result<Json<FeedResponse>, AppError> {
    let per_page = params.per_page.clamp(1, 200);
    let page = params.page.max(1);

    let rows = {
        let storage = state.core.storage.lock();
        storage.query_release_feed(
            params.repo.as_deref(),
            params.since.as_deref(),
            page,
            per_page,
        )
    };

    let items = rows.into_iter().map(row_to_feed_item).collect();
    Ok(Json(FeedResponse {
        page,
        per_page,
        items,
    }))
}

pub async fn get_changelog_repos(
    State(state): State<AppState>,
) -> Result<Json<Vec<RepoInfo>>, AppError> {
    let repos = {
        let storage = state.core.storage.lock();
        storage.query_release_repos()
    };

    let info = repos
        .into_iter()
        .map(|r| RepoInfo {
            repo: r.repo,
            latest_version: r.latest_version,
            latest_date: r.latest_date,
        })
        .collect();

    Ok(Json(info))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_semver() {
        assert_eq!(parse_semver("2.22.0"), Some((2, 22, 0)));
        assert_eq!(parse_semver("v2.11.0"), Some((2, 11, 0)));
        assert_eq!(parse_semver("1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_semver("bad"), None);
    }

    #[test]
    fn test_semver_cmp() {
        use std::cmp::Ordering::*;
        assert_eq!(semver_cmp("2.22.0", "2.11.0"), Greater);
        assert_eq!(semver_cmp("2.11.0", "2.22.0"), Less);
        assert_eq!(semver_cmp("2.11.0", "2.11.0"), Equal);
        assert_eq!(semver_cmp("3.0.0", "2.22.0"), Greater);
    }

    #[test]
    fn test_parse_changelog_version_headers() {
        let md = "# Changelog\n\n\
## [2.22.0] \u{2014} 2026-05-21 \u{2014} Wave C \u{2014} theme one\n\n\
## CO-134 \u{2014} Deployer adapter\n\n\
Some body text.\n\n\
## CO-259 \u{2014} Split sidebar\n\n\
## [2.21.0] \u{2014} 2026-05-20 \u{2014} Wave B \u{2014} theme two\n\n\
## CO-244 \u{2014} REPL\n";
        let versions = parse_changelog(md);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, "2.22.0");
        assert_eq!(versions[0].date, "2026-05-21");
        assert_eq!(versions[0].entries.len(), 2);
        assert_eq!(versions[0].entries[0].ticket, "CO-134");
        assert_eq!(versions[0].entries[1].ticket, "CO-259");
        assert_eq!(versions[1].version, "2.21.0");
        assert_eq!(versions[1].entries.len(), 1);
        assert_eq!(versions[1].entries[0].ticket, "CO-244");
    }

    #[test]
    fn test_parse_commit_subject_feat() {
        let (t, ticket, pr) =
            parse_commit_subject("feat(integrations): CO-243 \u{2014} VS Code extension (#75)")
                .unwrap();
        assert_eq!(t, "feat");
        assert_eq!(ticket, "CO-243");
        assert_eq!(pr, Some(75));
    }

    #[test]
    fn test_parse_commit_subject_refactor_no_pr() {
        let (t, ticket, pr) =
            parse_commit_subject("refactor(spa): CO-259 \u{2014} split sidebar").unwrap();
        assert_eq!(t, "refactor");
        assert_eq!(ticket, "CO-259");
        assert_eq!(pr, None);
    }

    #[test]
    fn test_parse_commit_subject_no_ticket() {
        assert!(parse_commit_subject("chore: bump deps").is_none());
    }

    #[test]
    fn test_version_range_filter() {
        let md = "## [2.22.0] \u{2014} 2026-05-21 \u{2014} t\n\n\
## CO-1 \u{2014} A\n\n\
## [2.21.0] \u{2014} 2026-05-20 \u{2014} t\n\n\
## CO-2 \u{2014} B\n\n\
## [2.11.0] \u{2014} 2026-04-01 \u{2014} t\n\n\
## CO-3 \u{2014} C\n";
        let mut versions = parse_changelog(md);

        versions.retain(|v| {
            semver_cmp(&v.version, "2.11.0") != std::cmp::Ordering::Less
                && semver_cmp(&v.version, "2.21.0") != std::cmp::Ordering::Greater
        });

        assert_eq!(versions.len(), 2);
        assert!(versions.iter().any(|v| v.version == "2.21.0"));
        assert!(versions.iter().any(|v| v.version == "2.11.0"));
    }

    #[test]
    fn test_enrich_from_cache_updates_type() {
        let mut versions = vec![VersionBlock {
            version: "2.22.0".to_string(),
            date: "2026-05-21".to_string(),
            tag_sha: None,
            theme: "test".to_string(),
            entries: vec![ChangelogEntry {
                ticket: "CO-134".to_string(),
                entry_type: "feat".to_string(),
                title: "original".to_string(),
                pr: None,
                pr_size: None,
                additions: None,
                deletions: None,
                commit_sha: None,
                author: None,
                spec_url: "/co/CO-134".to_string(),
            }],
        }];
        let cache = vec![ChangelogCacheRow {
            version: "2.22.0".to_string(),
            ticket: "CO-134".to_string(),
            entry_type: "refactor".to_string(),
            title: String::new(),
            pr_number: Some(73),
            pr_size: Some(842),
            additions: Some(800),
            deletions: Some(42),
            commit_sha: Some("abc123".to_string()),
            author: Some("yuri".to_string()),
        }];
        enrich_from_cache(&mut versions, &cache);
        let e = &versions[0].entries[0];
        assert_eq!(e.entry_type, "refactor");
        assert_eq!(e.pr, Some(73));
        assert_eq!(e.pr_size, Some(842));
        assert_eq!(e.commit_sha.as_deref(), Some("abc123"));
    }
}
