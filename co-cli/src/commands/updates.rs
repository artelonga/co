//! CO-404: `co updates` — release notes in the CLI.
//!
//! Renders release sections from the CHANGELOG embedded at compile time, so
//! the notes always match the installed binary. Newest first. A release
//! section starts at a `## [X.Y.Z] — date — theme` header and runs until the
//! next one; the `## CO-N — …` task entries inside stay at the same heading
//! level (the `[` distinguishes them).

use colored::Colorize;

const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// One release section of the changelog: header + body lines.
struct Release<'a> {
    header: &'a str,
    body: Vec<&'a str>,
}

/// Split the changelog into release sections, newest first (file order).
fn parse_releases(src: &str) -> Vec<Release<'_>> {
    let mut out: Vec<Release<'_>> = Vec::new();
    for line in src.lines() {
        if line.starts_with("## [") {
            out.push(Release {
                header: line.trim_start_matches("## ").trim(),
                body: Vec::new(),
            });
        } else if let Some(cur) = out.last_mut() {
            cur.body.push(line);
        }
    }
    // Trim trailing blank lines from each body.
    for r in &mut out {
        while r.body.last().is_some_and(|l| l.trim().is_empty()) {
            r.body.pop();
        }
    }
    out
}

fn print_release(release: &Release<'_>) {
    println!("{}", release.header.bold().cyan());
    for line in &release.body {
        if let Some(entry) = line.strip_prefix("## ") {
            println!("\n  {}", entry.bold());
        } else if let Some(why) = line.strip_prefix("### ") {
            println!("  {}", why.dimmed().italic());
        } else if line.trim().is_empty() {
            println!();
        } else {
            println!("  {line}");
        }
    }
    println!();
}

pub fn run(count: usize, all: bool) {
    let releases = parse_releases(CHANGELOG);
    if releases.is_empty() {
        println!("Nenhuma release encontrada no changelog embutido.");
        return;
    }

    if all {
        println!("{}\n", "CO — histórico de releases".bold().green());
        for r in &releases {
            println!("  {}", r.header.cyan());
        }
        println!(
            "\n{}",
            "co updates -n <N> mostra as N mais recentes em detalhe.".dimmed()
        );
        return;
    }

    println!("{}\n", "CO — novidades".bold().green());
    for r in releases.iter().take(count.max(1)) {
        print_release(r);
    }
    if releases.len() > count {
        println!(
            "{}",
            format!(
                "+{} releases anteriores — co updates --all para o histórico.",
                releases.len() - count
            )
            .dimmed()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Changelog

## [2.0.0] — 2026-06-01 — second
## CO-2 — did a thing
body line

### Why
because

## [1.0.0] — 2026-01-01 — first
## CO-1 — origin
";

    #[test]
    fn test_parse_releases_splits_on_bracket_headers_only() {
        let releases = parse_releases(SAMPLE);
        assert_eq!(releases.len(), 2);
        assert!(releases[0].header.starts_with("[2.0.0]"));
        assert!(releases[1].header.starts_with("[1.0.0]"));
        // Task entries stay in the body, not as section splits.
        assert!(releases[0].body.iter().any(|l| l.contains("CO-2")));
        assert!(releases[1].body.iter().any(|l| l.contains("CO-1")));
    }

    #[test]
    fn test_parse_releases_embedded_changelog_nonempty() {
        let releases = parse_releases(CHANGELOG);
        assert!(
            !releases.is_empty(),
            "embedded CHANGELOG must have releases"
        );
        // Newest-first: the first header carries the highest version.
        assert!(releases[0].header.starts_with('['));
    }

    #[test]
    fn test_parse_releases_trims_trailing_blanks() {
        let releases = parse_releases(SAMPLE);
        assert!(!releases[0].body.last().unwrap().trim().is_empty());
    }
}
