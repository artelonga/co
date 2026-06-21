//! CO-89: pure parser for `git log` output → [`CommitRecord`].
//!
//! This module shells out to nothing — it consumes a machine-readable string
//! produced by `git log` with a controlled `--pretty=format` and `--shortstat`,
//! so the whole git → entry pipeline is deterministically unit-testable without
//! a real repository or network. See [`GIT_LOG_FORMAT`] for the exact format.
//!
//! No `regex` dependency: the project does not pull in `regex`, so ticket-id /
//! conventional-commit-scope / shortstat extraction is hand-rolled below and
//! covered by the unit tests at the bottom of this file.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

/// ASCII Record Separator — prefixes each commit block in the git output.
pub const RS: char = '\u{1e}';
/// ASCII Unit Separator — separates fields within a commit block.
pub const US: char = '\u{1f}';

/// `--pretty=format` string fed to `git log`. Fields, in order:
/// `%H` sha · `%aI` author-date (strict ISO) · `%aN` author-name ·
/// `%aE` author-email · `%cN` committer-name · `%cE` committer-email ·
/// `%P` parents · `%s` subject · `%b` body.
///
/// Each record is prefixed by [`RS`]; fields are joined by [`US`]. The body
/// (`%b`) is multi-line and `--shortstat` appends its summary line *after* the
/// body, so the final `US`-field carries `body + shortstat` and is split apart
/// by [`parse_git_log`].
pub const GIT_LOG_FORMAT: &str = "\u{1e}%H\u{1f}%aI\u{1f}%aN\u{1f}%aE\u{1f}%cN\u{1f}%cE\u{1f}%P\u{1f}%s\u{1f}%b";

/// A single commit parsed from `git log`, ready to become a `commit` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitRecord {
    pub sha: String,
    /// Strict-ISO author date as emitted by `%aI`, e.g. `2026-04-26T23:00:00-03:00`.
    pub committed_at: String,
    /// Epoch nanoseconds (UTC) of `committed_at`.
    pub committed_at_ns: i64,
    /// UTC offset of the author's local time, formatted `±HH:MM` (e.g. `-03:00`).
    /// This is the *meaningful* zone for display per CO-89's timezone handling;
    /// a true IANA name can't be recovered from an offset, so the offset is kept.
    pub tz_offset: String,
    pub author_name: String,
    pub author_email: String,
    pub author_handle: String,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_handle: String,
    /// Parent SHAs (more than one ⇒ a merge commit).
    pub parents: Vec<String>,
    /// First line of the commit message.
    pub subject: String,
    /// Full commit message (subject + body), used as the entry body.
    pub message: String,
    /// Ticket id parsed from the subject (`feat(co-web): CO-65 …` → `CO-65`).
    pub parent_task: Option<String>,
    /// Conventional-commit scope(s) (`feat(co-web): …` → `["co-web"]`).
    pub tags: Vec<String>,
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
}

impl CommitRecord {
    /// Total lines touched (insertions + deletions).
    pub fn churn(&self) -> u32 {
        self.insertions + self.deletions
    }
}

/// Parse the full output of `git log <GIT_LOG_FORMAT> --shortstat <branch>`
/// into one [`CommitRecord`] per commit, newest-first (git's default order).
///
/// Malformed records (too few fields, unparseable date) are skipped rather than
/// failing the whole sync — a single weird commit must not block ingestion.
pub fn parse_git_log(raw: &str) -> Vec<CommitRecord> {
    raw.split(RS)
        .filter(|chunk| !chunk.trim().is_empty())
        .filter_map(parse_record)
        .collect()
}

fn parse_record(chunk: &str) -> Option<CommitRecord> {
    // Split into the 9 leading fields; the 9th (body) keeps embedded US-free
    // newlines and the trailing shortstat line.
    let mut parts = chunk.splitn(9, US);
    let sha = parts.next()?.trim().to_string();
    if sha.is_empty() {
        return None;
    }
    let committed_at = parts.next()?.trim().to_string();
    let author_name = parts.next()?.to_string();
    let author_email = parts.next()?.trim().to_string();
    let committer_name = parts.next()?.to_string();
    let committer_email = parts.next()?.trim().to_string();
    let parents_raw = parts.next()?.trim().to_string();
    let subject = parts.next()?.to_string();
    let body_and_stat = parts.next().unwrap_or("");

    let dt = DateTime::parse_from_rfc3339(&committed_at).ok()?;
    let committed_at_ns = dt.timestamp_nanos_opt()?;
    let tz_offset = format_offset(&dt);

    let parents: Vec<String> = parents_raw
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    let (body, stat) = split_body_and_shortstat(body_and_stat);
    let (files_changed, insertions, deletions) = parse_shortstat(stat);

    // Full message = subject + body (matching what a human sees in `git show`).
    let message = if body.trim().is_empty() {
        subject.clone()
    } else {
        format!("{}\n\n{}", subject.trim_end(), body.trim())
    };

    let parent_task = extract_ticket_id(&subject);
    let tags = extract_scopes(&subject);

    Some(CommitRecord {
        sha,
        committed_at,
        committed_at_ns,
        tz_offset,
        author_handle: handle_from_email(&author_email),
        author_name,
        author_email,
        committer_handle: handle_from_email(&committer_email),
        committer_name,
        committer_email,
        parents,
        subject,
        message,
        parent_task,
        tags,
        files_changed,
        insertions,
        deletions,
    })
}

/// Format a fixed offset as `±HH:MM`.
fn format_offset(dt: &DateTime<FixedOffset>) -> String {
    let secs = dt.offset().local_minus_utc();
    let sign = if secs < 0 { '-' } else { '+' };
    let abs = secs.abs();
    format!("{}{:02}:{:02}", sign, abs / 3600, (abs % 3600) / 60)
}

/// Split a `body + shortstat` blob into `(body, shortstat_line)`. The shortstat
/// is git's ` N files changed, …` summary, always the last non-empty line when
/// `--shortstat` is on; everything before it is the commit body. If no shortstat
/// line is present (merge/empty commits), the whole blob is the body.
fn split_body_and_shortstat(blob: &str) -> (String, &str) {
    let trimmed = blob.trim_end_matches(['\n', RS]);
    // The shortstat is the last line iff it looks like a shortstat.
    if let Some(idx) = trimmed.rfind('\n') {
        let last = &trimmed[idx + 1..];
        if looks_like_shortstat(last) {
            return (trimmed[..idx].to_string(), last);
        }
    } else if looks_like_shortstat(trimmed) {
        // Body was empty; the only line is the shortstat.
        return (String::new(), trimmed);
    }
    (trimmed.to_string(), "")
}

fn looks_like_shortstat(line: &str) -> bool {
    let t = line.trim_start();
    // " 3 files changed, …" / " 1 file changed, …"
    let mut chars = t.chars();
    let has_leading_digit = chars.next().is_some_and(|c| c.is_ascii_digit());
    has_leading_digit && t.contains("file") && t.contains("changed")
}

/// Parse ` N files changed, X insertions(+), Y deletions(-)` → `(N, X, Y)`.
/// Any missing clause (e.g. a doc-only commit with no deletions) defaults to 0.
pub fn parse_shortstat(line: &str) -> (u32, u32, u32) {
    let mut files = 0u32;
    let mut ins = 0u32;
    let mut del = 0u32;
    for clause in line.split(',') {
        let clause = clause.trim();
        let n: u32 = clause
            .split_whitespace()
            .next()
            .and_then(|tok| tok.parse().ok())
            .unwrap_or(0);
        if clause.contains("file") {
            files = n;
        } else if clause.contains("insertion") {
            ins = n;
        } else if clause.contains("deletion") {
            del = n;
        }
    }
    (files, ins, del)
}

/// Extract the first ticket id (`[A-Z]{2,}-\d+`, e.g. `CO-65`, `GP-8`) from a
/// commit subject. Generalized beyond `CO-` so any universe's convention works.
pub fn extract_ticket_id(subject: &str) -> Option<String> {
    let bytes = subject.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Start of a candidate: an uppercase ASCII letter at a word boundary.
        let prev_is_alnum = i > 0 && (bytes[i - 1].is_ascii_alphanumeric());
        if !prev_is_alnum && bytes[i].is_ascii_uppercase() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_uppercase() {
                i += 1;
            }
            let letters = i - start;
            if letters >= 2 && i < bytes.len() && bytes[i] == b'-' {
                let dash = i;
                i += 1;
                let digit_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i > digit_start {
                    return Some(subject[start..i].to_string());
                }
                i = dash + 1;
                continue;
            }
        }
        i += 1;
    }
    None
}

/// Extract conventional-commit scope(s) from a subject:
/// `feat(co-web): …` → `["co-web"]`, `chore(a,b): …` → `["a", "b"]`.
/// Returns empty when there's no `type(scope):` prefix.
pub fn extract_scopes(subject: &str) -> Vec<String> {
    // Only consider the prefix before the first ':'.
    let Some(colon) = subject.find(':') else {
        return Vec::new();
    };
    let prefix = &subject[..colon];
    let Some(open) = prefix.find('(') else {
        return Vec::new();
    };
    let Some(close_rel) = prefix[open + 1..].find(')') else {
        return Vec::new();
    };
    let inner = &prefix[open + 1..open + 1 + close_rel];
    inner
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Derive a stable handle from an author email.
///
/// * `yuri@artelonga.com.br` → `yuri`
/// * `198982+ArteLonga@users.noreply.github.com` → `artelonga` (GitHub noreply)
/// * `noreply@anthropic.com` → `claude` (the AI co-author identity)
pub fn handle_from_email(email: &str) -> String {
    let email = email.trim().trim_start_matches('<').trim_end_matches('>');
    if email.eq_ignore_ascii_case("noreply@anthropic.com") {
        return "claude".to_string();
    }
    let local = email.split('@').next().unwrap_or(email);
    // GitHub noreply form: `<id>+<login>` — keep the login.
    let local = local.rsplit('+').next().unwrap_or(local);
    let cleaned: String = local
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let handle = cleaned.trim_matches('-').to_ascii_lowercase();
    if handle.is_empty() {
        "unknown".to_string()
    } else {
        handle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_log() -> String {
        // Two commits, newest first, in the GIT_LOG_FORMAT + --shortstat shape.
        format!(
            "{RS}abc123{US}2026-04-26T23:00:00-03:00{US}Yuri{US}yuri@artelonga.com.br\
{US}Yuri{US}yuri@artelonga.com.br{US}def456{US}feat(co-web): CO-65 visibility on PUT\
{US}Adds visibility flag.\n\nDetails here.\n 3 files changed, 10 insertions(+), 2 deletions(-)\n\
{RS}def456{US}2026-04-25T09:30:00+09:00{US}Claude{US}noreply@anthropic.com\
{US}Claude{US}noreply@anthropic.com{US}{US}chore: initial commit{US}\n 1 file changed, 5 insertions(+)\n"
        )
    }

    #[test]
    fn parses_two_commits_with_fields() {
        let recs = parse_git_log(&sample_log());
        assert_eq!(recs.len(), 2);

        let c0 = &recs[0];
        assert_eq!(c0.sha, "abc123");
        assert_eq!(c0.author_handle, "yuri");
        assert_eq!(c0.committed_at, "2026-04-26T23:00:00-03:00");
        assert_eq!(c0.tz_offset, "-03:00");
        assert_eq!(c0.parents, vec!["def456".to_string()]);
        assert_eq!(c0.subject, "feat(co-web): CO-65 visibility on PUT");
        assert_eq!(c0.parent_task.as_deref(), Some("CO-65"));
        assert_eq!(c0.tags, vec!["co-web".to_string()]);
        assert_eq!(c0.files_changed, 3);
        assert_eq!(c0.insertions, 10);
        assert_eq!(c0.deletions, 2);
        assert!(c0.message.contains("Details here."));

        let c1 = &recs[1];
        assert_eq!(c1.author_handle, "claude");
        assert_eq!(c1.tz_offset, "+09:00");
        assert_eq!(c1.parent_task, None);
        assert_eq!(c1.tags, Vec::<String>::new());
        assert_eq!(c1.files_changed, 1);
        assert_eq!(c1.insertions, 5);
        assert_eq!(c1.deletions, 0);
        // Empty body ⇒ message == subject.
        assert_eq!(c1.message, "chore: initial commit");
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(parse_git_log("").is_empty());
        assert!(parse_git_log("   \n  ").is_empty());
    }

    #[test]
    fn ticket_id_extraction() {
        assert_eq!(extract_ticket_id("feat: CO-65 thing"), Some("CO-65".into()));
        assert_eq!(extract_ticket_id("GP-8 fix"), Some("GP-8".into()));
        assert_eq!(extract_ticket_id("no ticket here"), None);
        // Not a ticket: single uppercase letter, or lowercase.
        assert_eq!(extract_ticket_id("A-1 nope"), None);
        assert_eq!(extract_ticket_id("co-65 lower"), None);
        // First match wins.
        assert_eq!(
            extract_ticket_id("CO-1 and CO-2"),
            Some("CO-1".into())
        );
    }

    #[test]
    fn scope_extraction() {
        assert_eq!(extract_scopes("feat(co-web): x"), vec!["co-web"]);
        assert_eq!(extract_scopes("fix(a,b): x"), vec!["a", "b"]);
        assert_eq!(extract_scopes("chore: no scope"), Vec::<String>::new());
        assert_eq!(extract_scopes("no colon at all"), Vec::<String>::new());
        assert_eq!(extract_scopes("feat(core)!: breaking"), vec!["core"]);
    }

    #[test]
    fn shortstat_variants() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 10 insertions(+), 2 deletions(-)"),
            (3, 10, 2)
        );
        assert_eq!(
            parse_shortstat(" 1 file changed, 5 insertions(+)"),
            (1, 5, 0)
        );
        assert_eq!(
            parse_shortstat(" 2 files changed, 4 deletions(-)"),
            (2, 0, 4)
        );
        assert_eq!(parse_shortstat(""), (0, 0, 0));
    }

    #[test]
    fn handle_derivation() {
        assert_eq!(handle_from_email("yuri@artelonga.com.br"), "yuri");
        assert_eq!(handle_from_email("noreply@anthropic.com"), "claude");
        assert_eq!(
            handle_from_email("198982+ArteLonga@users.noreply.github.com"),
            "artelonga"
        );
        assert_eq!(handle_from_email("First.Last@x.com"), "first-last");
    }

    #[test]
    fn churn_sums_lines() {
        let recs = parse_git_log(&sample_log());
        assert_eq!(recs[0].churn(), 12);
    }
}
