//! CO-555: ingest a translator-produced language mirror as language-tagged entries.
//!
//! The modular translator (`scripts/i18n/translate.py`) renders a pt-canonical
//! markdown tree into a parallel `<out>/<lang>/...` mirror, keeping slugs and
//! forcing frontmatter `language: <lang>`. This module takes that mirror and
//! ingests it into a universe as **separate entries** at path `<lang>/<slug>`
//! (e.g. `en/sobre.md`), so `/sobre` ↔ `/en/sobre` is a slug-parallel switch.
//!
//! ## Design
//!
//! - **Idempotent.** Re-ingesting an unchanged mirror is a no-op: each page is
//!   keyed by its body hash against the existing entry; equal ⇒ `unchanged`.
//! - **Validated before ingest.** Each page is written to disk and run through
//!   [`co::validate::validate_file`]; a page with any *error*-severity issue is
//!   skipped (never indexed) and reported. Warnings (e.g. unknown `type`) pass.
//! - **Writes through the vault + repository, never raw SQLite.** Per-universe
//!   DBs are sharded under `universes/{ab}/{cd}/{key}/data.db`; we go through
//!   [`co::write_entry`] (file) + [`SqliteEntryRepository`] (index), exactly like
//!   the CO-416 translate route's persistence path.
//!
//! The translator-mirror entries are NEW paths (`en/…`), so they never collide
//! with the boot-time `include_str!` reseed of `index.md` / `content/*.md`.

use std::path::{Path, PathBuf};

use crate::repository::SqliteEntryRepository;

/// Outcome of ingesting a language mirror.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct IngestReport {
    /// Pages indexed for the first time.
    pub created: usize,
    /// Pages whose body changed and were re-indexed.
    pub updated: usize,
    /// Pages already present with an identical body (no-op).
    pub unchanged: usize,
    /// Pages skipped, with `(entry_path, reason)`.
    pub skipped: Vec<(String, String)>,
}

impl IngestReport {
    /// Total pages that resulted in an indexed entry (created + updated).
    pub fn indexed(&self) -> usize {
        self.created + self.updated
    }
}

/// Split a `---`-delimited frontmatter block from a markdown document.
/// Returns `(frontmatter_yaml, body)`; `("", whole)` when there is none.
fn split_frontmatter(md: &str) -> (&str, &str) {
    let Some(rest) = md.strip_prefix("---\n") else {
        return ("", md);
    };
    if let Some(end) = rest.find("\n---") {
        let body = rest[end + 4..].trim_start_matches(['\n', '\r']);
        return (&rest[..end], body);
    }
    ("", md)
}

/// Build the language-tagged entry path: `<lang>/<rel>` (forward slashes).
/// `rel` is normalized (leading slashes and `./` stripped).
pub fn lang_entry_path(lang: &str, rel: &str) -> String {
    let rel = rel.trim_start_matches('/').trim_start_matches("./");
    format!("{lang}/{rel}")
}

/// Parse one mirror page into an `Entry` at `<lang>/<rel>`, forcing
/// `language: <lang>` in the frontmatter (defensive — the translator already
/// sets it, but ingest is the trust boundary).
fn page_to_entry(lang: &str, rel: &str, md: &str) -> co::entry::Entry {
    let (fm_yaml, body) = split_frontmatter(md);
    let mut fm: serde_json::Value = serde_yaml::from_str::<serde_yaml::Value>(fm_yaml)
        .ok()
        .and_then(|v| serde_json::to_value(v).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = fm.as_object_mut() {
        obj.insert(
            "language".into(),
            serde_json::Value::String(lang.to_string()),
        );
    }
    crate::entry_index::make_entry(&lang_entry_path(lang, rel), fm, body)
}

/// Ingest a set of `(relative_path, markdown)` pages as `<lang>/<rel>` entries.
///
/// `universe_root` is the on-disk root of the universe (`universe_root(key)`);
/// `repo` wraps that universe's sharded connection. Returns a per-page report.
///
/// The validator runs on the written file before indexing; an error-level issue
/// removes the file and skips indexing (so a bad translation never poisons the
/// index, and `/en/<slug>` simply falls back to pt at the routing layer).
pub fn ingest_language_pages(
    repo: &SqliteEntryRepository,
    universe_root: &Path,
    universe_key: &str,
    lang: &str,
    pages: &[(String, String)],
) -> IngestReport {
    let mut report = IngestReport::default();
    let ctx = co::validate::ValidationContext::new(universe_root);

    for (rel, md) in pages {
        let entry = page_to_entry(lang, rel, md);
        let path = entry.path.clone();

        // Idempotency: identical body already indexed ⇒ no-op (skip disk + index).
        match repo.get(universe_key, &path) {
            Ok(Some(existing)) if existing.body_hash == entry.body_hash => {
                report.unchanged += 1;
                continue;
            }
            Ok(existing) => {
                let is_update = existing.is_some();
                // Write the file, then validate it on disk before indexing.
                if let Err(e) = co::write_entry(universe_root, &entry) {
                    report.skipped.push((path, format!("write failed: {e}")));
                    continue;
                }
                let abs = universe_root.join(&entry.path);
                let errors: Vec<String> = co::validate::validate_file(&abs, &ctx)
                    .into_iter()
                    .filter(|i| i.severity == co::validate::Severity::Error)
                    .map(|i| i.message)
                    .collect();
                if !errors.is_empty() {
                    let _ = std::fs::remove_file(&abs);
                    report.skipped.push((path, errors.join("; ")));
                    continue;
                }
                if let Err(e) = repo.upsert_entry(universe_key, &entry) {
                    report.skipped.push((path, format!("index failed: {e}")));
                    continue;
                }
                let _ = repo.upsert_dates(universe_key, &entry, None);
                if is_update {
                    report.updated += 1;
                } else {
                    report.created += 1;
                }
            }
            Err(e) => {
                report.skipped.push((path, format!("lookup failed: {e}")));
            }
        }
    }
    report
}

/// Walk a translator mirror directory (`<out>/<lang>`) and ingest every `*.md`
/// file as a `<lang>/<rel>` entry. Thin filesystem wrapper over
/// [`ingest_language_pages`] — this is what consumes the translator output
/// (`python3 scripts/i18n/translate.py --to <lang> --out <out>` ⇒ `<out>/<lang>`).
pub fn ingest_mirror_dir(
    repo: &SqliteEntryRepository,
    universe_root: &Path,
    universe_key: &str,
    lang: &str,
    mirror_dir: &Path,
) -> std::io::Result<IngestReport> {
    let mut pages: Vec<(String, String)> = Vec::new();
    collect_markdown(mirror_dir, mirror_dir, &mut pages)?;
    pages.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(ingest_language_pages(
        repo,
        universe_root,
        universe_key,
        lang,
        &pages,
    ))
}

/// Recursively collect `*.md` files under `dir`, keyed by their path relative
/// to `root`. Errors on individual entries are skipped silently.
fn collect_markdown(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, String)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let p = entry.path();
        if p.is_dir() {
            collect_markdown(root, &p, out)?;
        } else if p.extension().and_then(|e| e.to_str()) == Some("md")
            && let (Ok(rel), Ok(md)) = (p.strip_prefix(root), std::fs::read_to_string(&p))
        {
            out.push((rel.to_string_lossy().replace('\\', "/"), md));
        }
    }
    Ok(())
}

/// Helper used by tests/tools to point at a translator output root and ingest a
/// single language's mirror (`<out>/<lang>`).
pub fn mirror_lang_dir(out_root: &Path, lang: &str) -> PathBuf {
    out_root.join(lang)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn open_repo() -> (tempfile::TempDir, SqliteEntryRepository) {
        let dir = tempdir().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::universe_pool::UNIVERSE_SCHEMA_FOR_TEST)
            .unwrap();
        let repo = SqliteEntryRepository::new(Arc::new(Mutex::new(conn)));
        (dir, repo)
    }

    const EN_SOBRE: &str = "---\nslug: sobre\nlanguage: en\ntitle: About Co\ntype: page\n---\n\n# About Co\n\nHello.\n";

    #[test]
    fn lang_entry_path_prefixes_lang() {
        assert_eq!(lang_entry_path("en", "sobre.md"), "en/sobre.md");
        assert_eq!(lang_entry_path("en", "/index.md"), "en/index.md");
        assert_eq!(lang_entry_path("en", "content/x.md"), "en/content/x.md");
    }

    #[test]
    fn page_to_entry_forces_language_and_path() {
        let e = page_to_entry("en", "sobre.md", EN_SOBRE);
        assert_eq!(e.path, "en/sobre.md");
        assert_eq!(e.frontmatter["language"], "en");
        assert_eq!(e.frontmatter["title"], "About Co");
        assert!(e.body.contains("# About Co"));
    }

    #[test]
    fn ingest_is_idempotent() {
        let (dir, repo) = open_repo();
        let pages = vec![("sobre.md".to_string(), EN_SOBRE.to_string())];

        let r1 = ingest_language_pages(&repo, dir.path(), "u", "en", &pages);
        assert_eq!(r1.created, 1, "first ingest creates: {r1:?}");
        assert_eq!(r1.unchanged, 0);
        assert!(r1.skipped.is_empty(), "no skips: {:?}", r1.skipped);

        // Same content again ⇒ pure no-op.
        let r2 = ingest_language_pages(&repo, dir.path(), "u", "en", &pages);
        assert_eq!(r2.unchanged, 1, "re-ingest unchanged: {r2:?}");
        assert_eq!(r2.created, 0);
        assert_eq!(r2.updated, 0);

        // Changed body ⇒ update.
        let changed = vec![(
            "sobre.md".to_string(),
            EN_SOBRE.replace("Hello.", "Hello, world."),
        )];
        let r3 = ingest_language_pages(&repo, dir.path(), "u", "en", &changed);
        assert_eq!(r3.updated, 1, "changed body updates: {r3:?}");
    }

    #[test]
    fn ingest_skips_page_with_validation_error() {
        let (dir, repo) = open_repo();
        // No `language` field would normally error, but ingest forces it — so to
        // provoke an error we use a frontmatter that fails to parse the closing
        // marker (no closing `---`).
        let bad = "---\nslug: broken\ntitle: x\n(no closing fence)\n";
        let pages = vec![("broken.md".to_string(), bad.to_string())];
        let r = ingest_language_pages(&repo, dir.path(), "u", "en", &pages);
        // Forced language means even a frontmatter-less doc validates; this page
        // has an unparseable frontmatter block → validator error → skipped.
        assert_eq!(r.created, 0);
        assert_eq!(r.skipped.len(), 1, "expected one skip: {r:?}");
        assert_eq!(r.skipped[0].0, "en/broken.md");
        // The bad file must not survive on disk.
        assert!(!dir.path().join("en/broken.md").exists());
    }

    #[test]
    fn ingest_mirror_dir_walks_md_files() {
        let (dir, repo) = open_repo();
        let mirror = tempdir().unwrap();
        std::fs::create_dir_all(mirror.path().join("content")).unwrap();
        std::fs::write(mirror.path().join("index.md"), EN_SOBRE).unwrap();
        std::fs::write(mirror.path().join("content/sobre.md"), EN_SOBRE).unwrap();
        std::fs::write(mirror.path().join("ignore.txt"), "not markdown").unwrap();

        let r = ingest_mirror_dir(&repo, dir.path(), "u", "en", mirror.path()).unwrap();
        assert_eq!(r.created, 2, "two .md ingested, .txt ignored: {r:?}");
        assert!(repo.get("u", "en/index.md").unwrap().is_some());
        assert!(repo.get("u", "en/content/sobre.md").unwrap().is_some());
    }
}
