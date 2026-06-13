//! CO-416: structure-preserving content translation (pt-br ↔ en).
//!
//! This is **content** translation — the markdown body of an entry — not UI
//! i18n (which already exists via `setLang`). Given a `lang: pt` entry it
//! produces a sibling `lang: en` twin (and vice-versa) on demand, traceable
//! back to its source via `translated_from` + `translated_from_hash`.
//!
//! ## Correctness invariant (the critical AC)
//!
//! The translation MUST preserve:
//! - **fenced code blocks** (```…```) and inline code (`…`) — never translated
//! - **wikilinks** `[[key::path]]` / `[[path]]` — the link *target* is never
//!   translated (we keep the whole link verbatim for v1 to avoid breaking the
//!   relation graph)
//! - **frontmatter** — carried over structurally; only human-facing string
//!   fields (`title`) are translated, machine fields are untouched.
//!
//! The algorithm masks every protected span with an opaque sentinel before the
//! prose is handed to the backend, then restores the spans afterwards. The
//! backend therefore never sees code or link targets and cannot corrupt them,
//! regardless of how the model behaves.
//!
//! ## Backend
//!
//! [`TranslateBackend`] is the pluggable provider, selected by
//! `CO_TRANSLATE_BACKEND`:
//! - unset / `none` / `manual` → [`NoopBackend`] (returns "not configured",
//!   surfaced as HTTP 503 by the route)
//! - `llm` → [`LlmBackend`], which reuses the existing `infra::ai::AiRouter`.

use std::sync::Arc;

use async_trait::async_trait;

/// A supported content language.
pub const LANGS: [&str; 2] = ["pt", "en"];

/// Returns true if `lang` is a language we can translate to/from.
pub fn is_supported_lang(lang: &str) -> bool {
    LANGS.contains(&lang)
}

/// The opposite language (pt↔en).
pub fn opposite_lang(lang: &str) -> &'static str {
    if lang == "en" { "pt" } else { "en" }
}

// ---------------------------------------------------------------------------
// Backend abstraction
// ---------------------------------------------------------------------------

/// Pluggable translation backend. Implementations translate **prose only** —
/// the caller masks code/links/frontmatter before calling, so a backend never
/// needs to (and must not) reason about markdown structure.
#[async_trait]
pub trait TranslateBackend: Send + Sync {
    /// Translate `text` from language `from` to language `to`.
    ///
    /// `text` is plain prose with opaque sentinels (e.g. `⟦CO0⟧`) standing in
    /// for protected spans. Implementations MUST preserve any sentinel
    /// substrings verbatim.
    async fn translate(&self, text: &str, from: &str, to: &str) -> anyhow::Result<String>;

    /// `true` if this backend can actually translate. `false` means the route
    /// should answer 503 "translation backend not configured".
    fn is_configured(&self) -> bool;

    /// Stable identifier (`"noop"`, `"llm"`, `"mock"`).
    fn name(&self) -> &'static str;
}

/// Default backend: does nothing, reports unconfigured. Selected when
/// `CO_TRANSLATE_BACKEND` is unset/`none`/`manual`.
pub struct NoopBackend;

#[async_trait]
impl TranslateBackend for NoopBackend {
    async fn translate(&self, _text: &str, _from: &str, _to: &str) -> anyhow::Result<String> {
        anyhow::bail!("translation backend not configured (set CO_TRANSLATE_BACKEND=llm to enable)")
    }
    fn is_configured(&self) -> bool {
        false
    }
    fn name(&self) -> &'static str {
        "noop"
    }
}

/// LLM-backed translation, reusing the shared `infra::ai::AiRouter`.
///
/// The provider (`ollama` by default) is asked, with a strict preamble, to
/// translate the prose while leaving the `⟦CO…⟧` sentinels untouched. Because
/// protected spans are already masked out before we get here, even a sloppy
/// model cannot damage code or link targets.
pub struct LlmBackend {
    router: Arc<crate::infra::ai::AiRouter>,
    provider: String,
}

impl LlmBackend {
    pub fn new(router: Arc<crate::infra::ai::AiRouter>, provider: impl Into<String>) -> Self {
        Self {
            router,
            provider: provider.into(),
        }
    }
}

#[async_trait]
impl TranslateBackend for LlmBackend {
    async fn translate(&self, text: &str, from: &str, to: &str) -> anyhow::Result<String> {
        let from_name = lang_name(from);
        let to_name = lang_name(to);
        let prompt = format!(
            "You are a professional translator. Translate the following text from \
{from_name} to {to_name}.\n\
RULES:\n\
- Output ONLY the translation, with no preamble, notes, or quotes.\n\
- Preserve all markdown formatting (headings, lists, bold, links).\n\
- Any token of the form ⟦CO<number>⟧ is a placeholder — copy it EXACTLY, \
do not translate, reorder, or alter it.\n\
- Keep line breaks and paragraph structure identical.\n\n\
TEXT TO TRANSLATE:\n{text}"
        );
        let resp = self.router.query(&self.provider, &prompt).await?;
        Ok(resp.text.trim().to_string())
    }
    fn is_configured(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "llm"
    }
}

fn lang_name(lang: &str) -> &'static str {
    match lang {
        "pt" => "Brazilian Portuguese (pt-br)",
        "en" => "English",
        _ => "the target language",
    }
}

/// Build the configured backend from the environment.
///
/// - `CO_TRANSLATE_BACKEND=llm` → [`LlmBackend`] over the shared `AiRouter`
///   (provider from `CO_TRANSLATE_PROVIDER`, default `ollama`).
/// - anything else (unset / `none` / `manual`) → [`NoopBackend`].
pub fn backend_from_env(router: Arc<crate::infra::ai::AiRouter>) -> Arc<dyn TranslateBackend> {
    let secrets = crate::infra::secrets::global();
    match secrets.get_or("CO_TRANSLATE_BACKEND", "").as_str() {
        "llm" => {
            let provider = secrets.get_or("CO_TRANSLATE_PROVIDER", "ollama");
            Arc::new(LlmBackend::new(router, provider))
        }
        _ => Arc::new(NoopBackend),
    }
}

// ---------------------------------------------------------------------------
// Structure-preserving masking
// ---------------------------------------------------------------------------

/// A protected span that must survive translation verbatim.
#[derive(Debug, Clone, PartialEq)]
struct Mask {
    /// The exact original substring (code block, inline code, or wikilink).
    original: String,
}

/// The result of masking a body: prose with sentinels, plus the spans to
/// restore. Sentinels are `⟦CO<n>⟧` — non-ASCII brackets the model is told to
/// leave alone, and which never collide with normal markdown.
struct Masked {
    text: String,
    masks: Vec<Mask>,
}

fn sentinel(i: usize) -> String {
    format!("\u{27e6}CO{i}\u{27e7}")
}

/// Mask fenced code blocks, inline code spans, and wikilinks in `body`,
/// replacing each with an opaque sentinel. The returned spans are restored
/// after translation in the same order they were extracted.
fn mask_protected_spans(body: &str) -> Masked {
    let mut masks: Vec<Mask> = Vec::new();
    let mut out = String::with_capacity(body.len());

    let bytes = body.as_bytes();
    let mut i = 0usize;
    let n = bytes.len();

    while i < n {
        // Fenced code block: a line starting (after optional spaces) with ``` or ~~~.
        if at_line_start(body, i)
            && let Some(end) = fenced_block_extent(body, i)
        {
            push_mask(&mut out, &mut masks, &body[i..end]);
            i = end;
            continue;
        }

        let c = bytes[i];

        // Inline code span: `…` (single or multiple backticks, matched run).
        if c == b'`'
            && let Some(end) = inline_code_extent(body, i)
        {
            push_mask(&mut out, &mut masks, &body[i..end]);
            i = end;
            continue;
        }

        // Wikilink: [[ … ]] (never spanning a newline).
        if c == b'['
            && i + 1 < n
            && bytes[i + 1] == b'['
            && let Some(end) = wikilink_extent(body, i)
        {
            push_mask(&mut out, &mut masks, &body[i..end]);
            i = end;
            continue;
        }

        // Ordinary character — copy through (respecting UTF-8 char boundaries).
        let ch_len = utf8_len(c);
        out.push_str(&body[i..i + ch_len]);
        i += ch_len;
    }

    Masked { text: out, masks }
}

fn push_mask(out: &mut String, masks: &mut Vec<Mask>, original: &str) {
    out.push_str(&sentinel(masks.len()));
    masks.push(Mask {
        original: original.to_string(),
    });
}

/// Restore masked spans after translation. The sentinel token must be intact.
fn unmask(text: &str, masks: &[Mask]) -> String {
    let mut out = text.to_string();
    for (i, m) in masks.iter().enumerate() {
        out = out.replace(&sentinel(i), &m.original);
    }
    out
}

fn at_line_start(body: &str, i: usize) -> bool {
    i == 0 || body.as_bytes()[i - 1] == b'\n'
}

fn utf8_len(first_byte: u8) -> usize {
    if first_byte < 0x80 {
        1
    } else if first_byte >> 5 == 0b110 {
        2
    } else if first_byte >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

/// If a fenced code block opens at byte `start` (line start), return the byte
/// index just past its closing fence (or end of input if unterminated).
fn fenced_block_extent(body: &str, start: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    // Skip up to 3 leading spaces (CommonMark allows indentation).
    let mut p = start;
    let mut indent = 0;
    while p < bytes.len() && bytes[p] == b' ' && indent < 3 {
        p += 1;
        indent += 1;
    }
    if p >= bytes.len() {
        return None;
    }
    let fence_char = bytes[p];
    if fence_char != b'`' && fence_char != b'~' {
        return None;
    }
    let mut fence_len = 0;
    while p < bytes.len() && bytes[p] == fence_char {
        p += 1;
        fence_len += 1;
    }
    if fence_len < 3 {
        return None;
    }
    // Find the end of the opening line.
    let mut line_end = p;
    while line_end < bytes.len() && bytes[line_end] != b'\n' {
        line_end += 1;
    }
    // Scan subsequent lines for a closing fence of >= fence_len of the same char.
    let mut q = line_end + 1; // start of next line (or past end)
    while q <= bytes.len() {
        let mut ls = q;
        // optional indentation
        let mut ind = 0;
        while ls < bytes.len() && bytes[ls] == b' ' && ind < 3 {
            ls += 1;
            ind += 1;
        }
        let mut close_len = 0;
        let mut cp = ls;
        while cp < bytes.len() && bytes[cp] == fence_char {
            cp += 1;
            close_len += 1;
        }
        if close_len >= fence_len {
            // closing fence — consume to end of this line (and trailing newline)
            let mut ce = cp;
            while ce < bytes.len() && bytes[ce] != b'\n' {
                ce += 1;
            }
            if ce < bytes.len() {
                ce += 1; // include newline
            }
            return Some(ce);
        }
        // advance to next line
        let mut ne = q;
        while ne < bytes.len() && bytes[ne] != b'\n' {
            ne += 1;
        }
        if ne >= bytes.len() {
            // unterminated fence — treat the rest as the block
            return Some(bytes.len());
        }
        q = ne + 1;
    }
    Some(bytes.len())
}

/// Inline code span starting at `start` (a backtick). Returns the byte index
/// just past the matching closing run, or None if unterminated on the line.
fn inline_code_extent(body: &str, start: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut p = start;
    let mut open_len = 0;
    while p < bytes.len() && bytes[p] == b'`' {
        p += 1;
        open_len += 1;
    }
    // Find a closing run of exactly open_len backticks, not crossing a newline.
    let mut q = p;
    while q < bytes.len() {
        if bytes[q] == b'\n' {
            return None; // inline code does not span lines
        }
        if bytes[q] == b'`' {
            let mut run = 0;
            let mut r = q;
            while r < bytes.len() && bytes[r] == b'`' {
                r += 1;
                run += 1;
            }
            if run == open_len {
                return Some(r);
            }
            q = r;
        } else {
            q += 1;
        }
    }
    None
}

/// Wikilink `[[ … ]]` starting at `start`. Returns the byte index just past
/// the closing `]]`, or None if unterminated / spans a newline.
fn wikilink_extent(body: &str, start: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut p = start + 2; // past the opening [[
    while p + 1 < bytes.len() {
        if bytes[p] == b'\n' {
            return None;
        }
        if bytes[p] == b']' && bytes[p + 1] == b']' {
            return Some(p + 2);
        }
        p += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Public translation entry point
// ---------------------------------------------------------------------------

/// The structure-preserving result of translating an entry body.
#[derive(Debug, Clone, PartialEq)]
pub struct TranslatedBody {
    pub body: String,
}

/// Translate a markdown body from `from` to `to`, preserving code blocks,
/// inline code, and wikilinks. The backend only ever sees masked prose.
pub async fn translate_body(
    backend: &dyn TranslateBackend,
    body: &str,
    from: &str,
    to: &str,
) -> anyhow::Result<TranslatedBody> {
    let masked = mask_protected_spans(body);
    // If there's no prose at all (whole body is protected), skip the backend.
    let prose_is_only_sentinels = masked.text.contains('\u{27e6}')
        && masked.text.chars().all(|c| {
            c.is_whitespace()
                || c == '\u{27e6}'
                || c == '\u{27e7}'
                || c == 'C'
                || c == 'O'
                || c.is_ascii_digit()
        });
    let translated_prose = if masked.text.trim().is_empty() || prose_is_only_sentinels {
        masked.text.clone()
    } else {
        backend.translate(&masked.text, from, to).await?
    };
    let restored = unmask(&translated_prose, &masked.masks);
    Ok(TranslatedBody { body: restored })
}

/// Translate the human-facing string fields of frontmatter (currently only
/// `title`), carrying every other field through unchanged. Machine fields
/// (`type`, `lang`, ids, dates, refs) are never sent to the backend.
pub async fn translate_frontmatter_title(
    backend: &dyn TranslateBackend,
    frontmatter: &serde_json::Value,
    from: &str,
    to: &str,
) -> anyhow::Result<serde_json::Value> {
    let mut fm = frontmatter.clone();
    let title = fm
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|t| !t.trim().is_empty());
    if let Some(title) = title {
        let masked = mask_protected_spans(&title);
        let t = if masked.text.trim().is_empty() {
            masked.text.clone()
        } else {
            backend.translate(&masked.text, from, to).await?
        };
        let restored = unmask(t.trim(), &masked.masks);
        if let Some(obj) = fm.as_object_mut() {
            obj.insert("title".into(), serde_json::Value::String(restored));
        }
    }
    Ok(fm)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic test backend that transforms two known prose words and
    /// asserts it NEVER receives a code fence or wikilink (proving masking).
    struct WrapBackend;

    #[async_trait]
    impl TranslateBackend for WrapBackend {
        async fn translate(&self, text: &str, _from: &str, _to: &str) -> anyhow::Result<String> {
            assert!(
                !text.contains("```"),
                "backend saw a code fence — masking failed"
            );
            assert!(
                !text.contains("[["),
                "backend saw a wikilink — masking failed"
            );
            Ok(text.replace("Olá", "Hello").replace("mundo", "world"))
        }
        fn is_configured(&self) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "wrap"
        }
    }

    const FIXTURE: &str = "Olá mundo. Veja `co run` aqui.\n\n\
```rust\n\
fn main() { println!(\"Olá mundo\"); }\n\
```\n\n\
Link para [[topologia::conceitos/ogunte.md]] e fim.\n";

    #[tokio::test]
    async fn preserves_code_blocks_inline_code_and_wikilinks() {
        let out = translate_body(&WrapBackend, FIXTURE, "pt", "en")
            .await
            .unwrap()
            .body;

        // Fenced code block survives byte-for-byte, including the Portuguese
        // string literal inside it (code is NEVER translated).
        assert!(
            out.contains("fn main() { println!(\"Olá mundo\"); }"),
            "code block was altered:\n{out}"
        );
        assert!(out.contains("```rust"), "code fence missing:\n{out}");

        // Inline code survives.
        assert!(out.contains("`co run`"), "inline code altered:\n{out}");

        // Wikilink target survives verbatim (relation graph integrity).
        assert!(
            out.contains("[[topologia::conceitos/ogunte.md]]"),
            "wikilink altered:\n{out}"
        );

        // Prose WAS translated.
        assert!(out.contains("Hello world."), "prose not translated:\n{out}");
    }

    #[tokio::test]
    async fn frontmatter_machine_fields_untouched_title_translated() {
        let fm = serde_json::json!({
            "type": "nota",
            "lang": "pt",
            "title": "Olá mundo",
            "due_at": "2026-06-12",
            "ref": "topologia::x.md",
        });
        let out = translate_frontmatter_title(&WrapBackend, &fm, "pt", "en")
            .await
            .unwrap();
        assert_eq!(out["type"], "nota");
        assert_eq!(out["lang"], "pt"); // route flips this later, not the translator
        assert_eq!(out["due_at"], "2026-06-12");
        assert_eq!(out["ref"], "topologia::x.md");
        assert_eq!(out["title"], "Hello world");
    }

    #[tokio::test]
    async fn idempotent_masking_round_trip_with_identity_passthrough() {
        // A backend that returns prose unchanged ⇒ output equals input exactly.
        struct Identity;
        #[async_trait]
        impl TranslateBackend for Identity {
            async fn translate(&self, t: &str, _f: &str, _to: &str) -> anyhow::Result<String> {
                Ok(t.to_string())
            }
            fn is_configured(&self) -> bool {
                true
            }
            fn name(&self) -> &'static str {
                "id"
            }
        }
        let out = translate_body(&Identity, FIXTURE, "pt", "en")
            .await
            .unwrap()
            .body;
        assert_eq!(
            out, FIXTURE,
            "identity translate must be a perfect round-trip"
        );
    }

    #[tokio::test]
    async fn body_that_is_only_code_skips_backend() {
        // Whole body is a protected span ⇒ backend.translate is not called.
        let only_code = "```\nrm -rf /\n```\n";
        let out = translate_body(&WrapBackend, only_code, "pt", "en")
            .await
            .unwrap()
            .body;
        assert_eq!(out, only_code);
    }

    #[tokio::test]
    async fn noop_backend_errors_and_is_unconfigured() {
        assert!(!NoopBackend.is_configured());
        let err = translate_body(&NoopBackend, "olá", "pt", "en")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not configured"));
    }

    #[test]
    fn opposite_lang_flips() {
        assert_eq!(opposite_lang("pt"), "en");
        assert_eq!(opposite_lang("en"), "pt");
    }

    #[test]
    fn nested_backticks_inline_code_matched() {
        // ``a `b` c`` is a single inline span (double-backtick delimited).
        let masked = mask_protected_spans("x ``a `b` c`` y");
        assert_eq!(masked.masks.len(), 1);
        assert_eq!(masked.masks[0].original, "``a `b` c``");
    }
}
