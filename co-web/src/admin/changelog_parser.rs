//! CO-334: Keep-a-Changelog parser for cross-repo changelog aggregation.
//!
//! Parses the `## [N.N.N] — YYYY-MM-DD — optional theme` format used by all
//! sister repos (ArteLonga, Quilombo, Yggdrasil, RFQ, CO).

use chrono::NaiveDate;

/// " — " separator (space + U+2014 em-dash + space)
const SEP: &str = " \u{2014} ";

/// One parsed release from a Keep-a-Changelog formatted file.
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseNote {
    pub repo: String,
    pub version: String,
    pub date: NaiveDate,
    pub theme: Option<String>,
    pub body_md: String,
    pub body_text: String,
}

/// Parse a Keep-a-Changelog formatted markdown string.
///
/// Header format: `## [N.N.N] — YYYY-MM-DD` or `## [N.N.N] — YYYY-MM-DD — theme`.
/// `[Unreleased]` and headers without a parseable date are skipped.
/// Returns one `ReleaseNote` per released version, in document order.
pub fn parse_keep_a_changelog(repo: &str, content: &str) -> Vec<ReleaseNote> {
    let mut notes: Vec<ReleaseNote> = Vec::new();
    // (version, date, theme, accumulated body lines)
    let mut current: Option<(String, NaiveDate, Option<String>, Vec<String>)> = None;

    for line in content.lines() {
        // Version header: ## [2.14.0] — 2026-05-01 — optional theme
        if let Some(rest) = line.strip_prefix("## [")
            && let Some(bracket_end) = rest.find(']')
        {
            let version = rest[..bracket_end].to_string();
            // After "]" looks like " — YYYY-MM-DD" or " — YYYY-MM-DD — theme"
            let after = &rest[bracket_end + 1..];
            let mut parts = after.splitn(3, SEP);
            let _leading = parts.next();
            let date_str = parts.next().unwrap_or("").trim();
            let theme = parts
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                if let Some((ver, prev_date, prev_theme, body_lines)) = current.take() {
                    notes.push(flush(repo, ver, prev_date, prev_theme, body_lines));
                }
                current = Some((version, date, theme, Vec::new()));
            }
            continue;
        }

        if let Some((_, _, _, ref mut lines)) = current {
            lines.push(line.to_string());
        }
    }

    if let Some((ver, date, theme, body_lines)) = current {
        notes.push(flush(repo, ver, date, theme, body_lines));
    }

    notes
}

fn flush(
    repo: &str,
    version: String,
    date: NaiveDate,
    theme: Option<String>,
    body_lines: Vec<String>,
) -> ReleaseNote {
    let body_md = body_lines.join("\n").trim().to_string();
    let body_text = strip_markdown(&body_md);
    ReleaseNote {
        repo: repo.to_string(),
        version,
        date,
        theme,
        body_md,
        body_text,
    }
}

/// Strip markdown formatting for plain-text search storage.
pub fn strip_markdown(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    for line in md.lines() {
        // Strip leading # characters (headings)
        let line = line.trim_start_matches('#').trim();
        // Remove bold/italic markers
        let line = line.replace("**", "").replace('*', "");
        // Remove inline code backticks
        let line = line.replace('`', "");
        // Remove link syntax [text](url) → text
        let line = strip_md_links(&line);
        if !line.trim().is_empty() {
            out.push_str(line.trim());
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

fn strip_md_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            let mut text = String::new();
            for c in chars.by_ref() {
                if c == ']' {
                    break;
                }
                text.push(c);
            }
            // Skip (url) part if present
            if chars.peek() == Some(&'(') {
                chars.next();
                for c in chars.by_ref() {
                    if c == ')' {
                        break;
                    }
                }
            }
            out.push_str(&text);
        } else {
            out.push(ch);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests — 5 fixtures: ArteLonga, Quilombo, Yggdrasil, RFQ, CO
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Actual em-dash separator used in all changelogs.
    const S: &str = " \u{2014} ";

    fn h(ver: &str, date: &str, theme: Option<&str>) -> String {
        match theme {
            Some(t) => format!("## [{ver}]{S}{date}{S}{t}"),
            None => format!("## [{ver}]{S}{date}"),
        }
    }

    // ── Fixture 1: ArteLonga ─────────────────────────────────────────────────

    #[test]
    fn test_parse_artelonga() {
        let md = format!(
            "# Changelog\n\n{}\n\n### Added\n- feat: cross-repo viewer\n\n### Fixed\n- fix: pagination\n\n{}\n\n### Added\n- feat: initial release\n",
            h("0.14.0", "2026-05-20", Some("Phase C wave 4-8 close")),
            h("0.13.0", "2026-04-10", Some("initial")),
        );
        let notes = parse_keep_a_changelog("artelonga", &md);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].version, "0.14.0");
        assert_eq!(notes[0].repo, "artelonga");
        assert_eq!(notes[0].date.to_string(), "2026-05-20");
        assert_eq!(notes[0].theme.as_deref(), Some("Phase C wave 4-8 close"));
        assert!(notes[0].body_md.contains("cross-repo viewer"));
        assert_eq!(notes[1].version, "0.13.0");
    }

    // ── Fixture 2: Quilombo (no theme, Portuguese body) ───────────────────────

    #[test]
    fn test_parse_quilombo() {
        let md = format!(
            "{}\n\n### Changed\n- fix(auth): session token expiry bug\n\n{}\n\n### Added\n- feat(events): new event type\n",
            h("0.12.7", "2026-05-24", None),
            h("0.12.6", "2026-05-20", None),
        );
        let notes = parse_keep_a_changelog("quilomboaraucaria", &md);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].version, "0.12.7");
        assert_eq!(notes[0].repo, "quilomboaraucaria");
        assert_eq!(notes[0].date.to_string(), "2026-05-24");
        assert!(notes[0].theme.is_none(), "Quilombo entries have no theme");
        assert!(notes[0].body_md.contains("session token expiry bug"));
    }

    // ── Fixture 3: Yggdrasil ─────────────────────────────────────────────────

    #[test]
    fn test_parse_yggdrasil() {
        let md = format!(
            "{}\n\n### Added\n- feat: Relic theme across platform\n\n{}\n\n### Fixed\n- fix: minor UI glitch\n",
            h(
                "1.3.0",
                "2026-06-01",
                Some("\u{cd}ndice /universos filtr\u{e1}vel")
            ),
            h("1.2.0", "2026-05-20", Some("patch")),
        );
        let notes = parse_keep_a_changelog("yggdrasil", &md);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].version, "1.3.0");
        assert_eq!(notes[0].date.to_string(), "2026-06-01");
        assert!(notes[0].body_md.contains("Relic theme"));
        assert_eq!(notes[1].version, "1.2.0");
    }

    // ── Fixture 4: RFQ Gateway ────────────────────────────────────────────────

    #[test]
    fn test_parse_rfq() {
        let md = format!(
            "{}\n\n### Added\n- feat(batch): parallel trade execution\n- feat(api): new endpoints\n\n{}\n\n### Fixed\n- fix(gateway): connection pool exhaustion\n",
            h("0.6.0", "2026-05-20", Some("Phase C close")),
            h("0.5.9", "2026-05-10", Some("stability")),
        );
        let notes = parse_keep_a_changelog("rfq", &md);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].version, "0.6.0");
        assert_eq!(notes[0].repo, "rfq");
        assert!(notes[0].body_md.contains("parallel trade execution"));
        assert_eq!(notes[1].version, "0.5.9");
    }

    // ── Fixture 5: CO (multi-dash theme, ticket-style body) ──────────────────

    #[test]
    fn test_parse_co() {
        // CO's theme itself contains em-dashes; splitn(3) must capture the full theme.
        let md = format!(
            "{}\n\n## CO-332 \u{2014} External assistant\n## CO-333 \u{2014} Feedback system\n\n{}\n\n## CO-330 \u{2014} local_repo_path\n",
            h(
                "2.36.0",
                "2026-06-01",
                Some("yuri vision Wave 2/3 \u{2014} LLM trait + tools as git repos")
            ),
            h("2.35.0", "2026-05-21", Some("Wave C")),
        );
        let notes = parse_keep_a_changelog("co", &md);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].version, "2.36.0");
        assert_eq!(notes[0].date.to_string(), "2026-06-01");
        // Theme with embedded em-dash must be preserved intact
        assert!(notes[0].theme.as_deref().unwrap().contains("LLM trait"));
        assert!(notes[0].body_md.contains("CO-332"));
        assert_eq!(notes[1].version, "2.35.0");
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn test_skip_unreleased() {
        let md = format!(
            "## [Unreleased]\n\nSome unreleased stuff\n\n{}\n\n### Added\n- feat: first release\n",
            h("1.0.0", "2026-01-01", Some("initial")),
        );
        let notes = parse_keep_a_changelog("test", &md);
        assert_eq!(
            notes.len(),
            1,
            "[Unreleased] has no date and must be skipped"
        );
        assert_eq!(notes[0].version, "1.0.0");
    }

    #[test]
    fn test_strip_markdown_removes_markers() {
        let md = "### Added\n- **Bold** item with `code`\n- [link text](https://example.com)";
        let text = strip_markdown(md);
        assert!(!text.contains('#'));
        assert!(!text.contains("**"));
        assert!(!text.contains('`'));
        assert!(text.contains("Bold"));
        assert!(text.contains("link text"));
        assert!(!text.contains("https://"));
    }
}
