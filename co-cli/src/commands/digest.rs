//! `co universe digest` — deterministic, recursive, token-bounded universe summaries (CO-428).
//!
//! ## Why this exists
//!
//! Every co-auto / Claude session pays for the context it loads. The biggest
//! lever is **prompt caching**: a stable prefix that is byte-identical across
//! tasks is billed once (cache-write) and then served from cache (≈10× cheaper).
//! Verbose, hand-written universe overviews defeat this — they drift, reorder,
//! and carry timestamps, so every session pays the full uncached price.
//!
//! `co universe digest` replaces that prose with a **generated, deterministic**
//! block: same DB state → same bytes, with a fixed key-sorted ordering and no
//! timestamps or random iteration order. That makes it usable as a cacheable
//! prefix shared across every task in the same universe.
//!
//! ## Cache-friendly layer ordering
//!
//! The digest emits layers stable-first, volatile-last so the longest possible
//! prefix stays byte-identical between runs:
//!
//! 1. **Architecture banner** (constant string) — never changes.
//! 2. **Universe forest** (key, name, purpose, parent, children, content-type
//!    counts) — changes only when universes are added/renamed/re-parented.
//! 3. **Per-universe counts** (entries, tasks-by-status) — the most volatile
//!    layer, emitted last within each block.
//!
//! Counts are the only fast-moving data; putting them at the tail of each
//! block means edits to one universe's task counts do not invalidate the
//! cached prefix of the universes printed before it.
//!
//! ## Token bound
//!
//! Each universe block targets ≤200 tokens (estimated as `chars / 4`). The
//! purpose line is truncated and counts are compact precisely so the bound
//! holds; `digest` prints a warning to stderr if any block exceeds it, but
//! still emits the (deterministic) content.
//!
//! ## Source of truth
//!
//! All data comes from the local CO registry (`Storage` / SQLite) — never a
//! hardcoded universe→repo mapping (CO-424 constraint).

use co_web::storage::Storage;
use std::collections::BTreeMap;

/// Output format for the digest.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DigestFormat {
    Markdown,
    Json,
}

impl DigestFormat {
    pub fn parse(s: &str) -> Self {
        match s {
            "json" => DigestFormat::Json,
            _ => DigestFormat::Markdown,
        }
    }
}

/// Per-universe metadata extracted deterministically from the registry.
struct UniverseDigest {
    key: String,
    name: String,
    /// 1–2 sentence purpose, derived from the description (truncated).
    purpose: String,
    parent: Option<String>,
    /// Direct children, sorted by key.
    children: Vec<String>,
    pages: i64,
    tasks: i64,
    projects: i64,
    /// Task counts grouped by status, sorted by status key.
    tasks_by_status: BTreeMap<String, i64>,
    /// Distinct content types ("modelos") present, sorted.
    content_types: Vec<String>,
}

/// Token estimate (chars / 4) for a block of text.
fn estimate_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

/// Per-universe token bound (CO-428 acceptance criterion).
const TOKEN_BUDGET: usize = 200;

/// Truncate a description to a short, single-line purpose.
/// Deterministic: collapses whitespace and cuts at a fixed character budget on
/// a word boundary.
fn purpose_from_description(description: &str) -> String {
    let collapsed = description.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX: usize = 160;
    if collapsed.chars().count() <= MAX {
        return collapsed;
    }
    let mut out = String::new();
    for word in collapsed.split(' ') {
        if out.chars().count() + word.chars().count() + 1 > MAX {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out.push('…');
    out
}

/// Collect deterministic digest data for a single universe from storage.
fn collect_one(storage: &Storage, key: &str, children: &[String]) -> Option<UniverseDigest> {
    let universe = storage.get_universe(key)?;
    let (pages, tasks, projects) = storage.count_universe_entries(key);

    // Per-universe DB queries: task-by-status and distinct content types.
    let mut tasks_by_status: BTreeMap<String, i64> = BTreeMap::new();
    let mut content_types: Vec<String> = Vec::new();
    let conn = storage.universe_conn(key);
    if let Ok(guard) = conn.lock() {
        // Task counts grouped by frontmatter status (json_extract).
        if let Ok(mut stmt) = guard.prepare(
            "SELECT COALESCE(json_extract(frontmatter_json, '$.status'), 'unset') AS s, COUNT(*) \
             FROM entries WHERE entry_type = 'task' GROUP BY s ORDER BY s",
        ) && let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            for r in rows.flatten() {
                tasks_by_status.insert(r.0, r.1);
            }
        }
        // Distinct content types ("modelos") in use, sorted.
        if let Ok(mut stmt) =
            guard.prepare("SELECT DISTINCT entry_type FROM entries ORDER BY entry_type")
            && let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0))
        {
            content_types = rows.flatten().collect();
        }
    }

    let mut sorted_children = children.to_vec();
    sorted_children.sort();

    Some(UniverseDigest {
        key: universe.key.clone(),
        name: universe.name,
        purpose: purpose_from_description(&universe.description),
        parent: universe.parent_key,
        children: sorted_children,
        pages,
        tasks,
        projects,
        tasks_by_status,
        content_types,
    })
}

/// Render a single universe block as Markdown. Stable bytes for stable input.
/// Counts (the volatile layer) are emitted last within the block.
fn render_md_block(d: &UniverseDigest, depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut s = String::new();
    // Stable identity layer.
    s.push_str(&format!("{}- **{}** — {}\n", indent, d.key, d.name));
    if !d.purpose.is_empty() {
        s.push_str(&format!("{}  purpose: {}\n", indent, d.purpose));
    }
    if let Some(p) = &d.parent {
        s.push_str(&format!("{}  parent: {}\n", indent, p));
    }
    if !d.children.is_empty() {
        s.push_str(&format!(
            "{}  children: {}\n",
            indent,
            d.children.join(", ")
        ));
    }
    if !d.content_types.is_empty() {
        s.push_str(&format!(
            "{}  models: {}\n",
            indent,
            d.content_types.join(", ")
        ));
    }
    // Volatile counts layer — last.
    let status_str = if d.tasks_by_status.is_empty() {
        String::new()
    } else {
        let parts: Vec<String> = d
            .tasks_by_status
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        format!(" [{}]", parts.join(" "))
    };
    s.push_str(&format!(
        "{}  counts: pages={} projects={} tasks={}{}\n",
        indent, d.pages, d.projects, d.tasks, status_str
    ));
    s
}

/// Build the ordered forest: roots first (sorted by key), each followed by its
/// subtree (DFS, children sorted by key) up to `max_depth`.
fn ordered_forest(
    digests: &BTreeMap<String, UniverseDigest>,
    start: Option<&str>,
    max_depth: usize,
) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let roots: Vec<String> = match start {
        Some(k) => {
            if digests.contains_key(k) {
                vec![k.to_string()]
            } else {
                vec![]
            }
        }
        None => digests
            .values()
            .filter(|d| match &d.parent {
                // A universe is a root if it has no parent, or its parent is
                // not in our set (orphan parent → treat as root).
                Some(p) => !digests.contains_key(p),
                None => true,
            })
            .map(|d| d.key.clone())
            .collect(),
    };
    // `roots` derived from a BTreeMap is already key-sorted; sort defensively.
    let mut roots = roots;
    roots.sort();
    for r in roots {
        push_subtree(digests, &r, 0, max_depth, &mut out);
    }
    out
}

fn push_subtree(
    digests: &BTreeMap<String, UniverseDigest>,
    key: &str,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<(String, usize)>,
) {
    let Some(d) = digests.get(key) else { return };
    out.push((key.to_string(), depth));
    if depth >= max_depth {
        return;
    }
    // Sort defensively so traversal order is deterministic regardless of how
    // `children` was populated (production `collect_one` already sorts).
    let mut kids = d.children.clone();
    kids.sort();
    for child in &kids {
        if digests.contains_key(child) {
            push_subtree(digests, child, depth + 1, max_depth, out);
        }
    }
}

/// Architecture banner — constant, stable prefix (cacheable, never changes).
const ARCH_BANNER: &str = "# CO universe digest\n\
\n\
CO is a graph-based content platform. A *universe* is a content workspace\n\
(markdown-canonical). Universes nest via `parent_key` (a parent owns child\n\
universes). This digest is generated from the local registry — deterministic\n\
and cache-friendly. Layers below are ordered stable-first, counts last.\n";

/// Run the digest command.
pub fn run(key: Option<String>, depth: usize, format: DigestFormat, data_dir: String) {
    let storage = Storage::new(&data_dir);

    // 1. Build child adjacency from parent_key over ALL universes (registry).
    let all_keys = {
        let mut k = storage.all_universe_keys();
        k.sort();
        k.dedup();
        k
    };
    let mut children_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for k in &all_keys {
        if let Some(u) = storage.get_universe(k)
            && let Some(p) = &u.parent_key
        {
            children_of.entry(p.clone()).or_default().push(k.clone());
        }
    }

    // 2. Collect per-universe digest data.
    let mut digests: BTreeMap<String, UniverseDigest> = BTreeMap::new();
    for k in &all_keys {
        let empty = Vec::new();
        let kids = children_of.get(k).unwrap_or(&empty);
        if let Some(d) = collect_one(&storage, k, kids) {
            digests.insert(k.clone(), d);
        }
    }

    // 3. Determine the ordered forest (DFS, key-sorted).
    let order = ordered_forest(&digests, key.as_deref(), depth);

    if order.is_empty() {
        match key {
            Some(k) => eprintln!("No universe found with key '{}'.", k),
            None => eprintln!("No universes registered in {}.", data_dir),
        }
        return;
    }

    match format {
        DigestFormat::Json => emit_json(&digests, &order),
        DigestFormat::Markdown => emit_markdown(&digests, &order),
    }
}

fn emit_markdown(digests: &BTreeMap<String, UniverseDigest>, order: &[(String, usize)]) {
    // Stable prefix: architecture banner.
    let mut out = String::new();
    out.push_str(ARCH_BANNER);
    out.push('\n');
    out.push_str("## Universes\n\n");

    for (k, depth) in order {
        if let Some(d) = digests.get(k) {
            let block = render_md_block(d, *depth);
            let tokens = estimate_tokens(&block);
            if tokens > TOKEN_BUDGET {
                eprintln!(
                    "warning: digest for '{}' is ~{} tokens (>{} budget)",
                    k, tokens, TOKEN_BUDGET
                );
            }
            out.push_str(&block);
        }
    }
    print!("{}", out);
}

fn emit_json(digests: &BTreeMap<String, UniverseDigest>, order: &[(String, usize)]) {
    // Deterministic hand-rolled JSON (BTreeMap order, no HashMap, no timestamps).
    let mut out = String::from("[\n");
    let mut first = true;
    for (k, depth) in order {
        let Some(d) = digests.get(k) else { continue };
        if !first {
            out.push_str(",\n");
        }
        first = false;
        let status_obj = if d.tasks_by_status.is_empty() {
            "{}".to_string()
        } else {
            let parts: Vec<String> = d
                .tasks_by_status
                .iter()
                .map(|(s, c)| format!("{}:{}", json_str(s), c))
                .collect();
            format!("{{{}}}", parts.join(","))
        };
        let children: Vec<String> = d.children.iter().map(|c| json_str(c)).collect();
        let models: Vec<String> = d.content_types.iter().map(|c| json_str(c)).collect();
        out.push_str(&format!(
            "  {{\"key\":{},\"name\":{},\"purpose\":{},\"parent\":{},\"depth\":{},\
\"children\":[{}],\"models\":[{}],\"pages\":{},\"projects\":{},\"tasks\":{},\"tasks_by_status\":{}}}",
            json_str(&d.key),
            json_str(&d.name),
            json_str(&d.purpose),
            d.parent
                .as_deref()
                .map(json_str)
                .unwrap_or_else(|| "null".to_string()),
            depth,
            children.join(","),
            models.join(","),
            d.pages,
            d.projects,
            d.tasks,
            status_obj,
        ));
    }
    out.push_str("\n]\n");
    print!("{}", out);
}

/// Minimal deterministic JSON string escaping.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_chars_over_4() {
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn purpose_collapses_whitespace() {
        assert_eq!(purpose_from_description("a   b\n\nc"), "a b c".to_string());
    }

    #[test]
    fn purpose_truncates_long_text() {
        let long = "word ".repeat(100);
        let p = purpose_from_description(&long);
        assert!(p.chars().count() <= 161); // 160 + ellipsis
        assert!(p.ends_with('…'));
    }

    #[test]
    fn format_parse_defaults_to_markdown() {
        assert!(DigestFormat::parse("md") == DigestFormat::Markdown);
        assert!(DigestFormat::parse("bogus") == DigestFormat::Markdown);
        assert!(DigestFormat::parse("json") == DigestFormat::Json);
    }

    #[test]
    fn json_str_escapes() {
        assert_eq!(json_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_str("a\nb"), "\"a\\nb\"");
    }

    fn d(key: &str, parent: Option<&str>, children: &[&str]) -> UniverseDigest {
        UniverseDigest {
            key: key.to_string(),
            name: format!("Name {}", key),
            purpose: "p".to_string(),
            parent: parent.map(|s| s.to_string()),
            children: children.iter().map(|s| s.to_string()).collect(),
            pages: 0,
            tasks: 0,
            projects: 0,
            tasks_by_status: BTreeMap::new(),
            content_types: vec![],
        }
    }

    fn tree() -> BTreeMap<String, UniverseDigest> {
        // root -> a, b ; a -> a1
        let mut m = BTreeMap::new();
        m.insert("root".to_string(), d("root", None, &["b", "a"]));
        m.insert("a".to_string(), d("a", Some("root"), &["a1"]));
        m.insert("b".to_string(), d("b", Some("root"), &[]));
        m.insert("a1".to_string(), d("a1", Some("a"), &[]));
        m
    }

    #[test]
    fn forest_is_dfs_key_sorted() {
        let m = tree();
        let order = ordered_forest(&m, None, 10);
        let keys: Vec<&str> = order.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["root", "a", "a1", "b"]);
        // depths
        let depths: Vec<usize> = order.iter().map(|(_, dp)| *dp).collect();
        assert_eq!(depths, vec![0, 1, 2, 1]);
    }

    #[test]
    fn forest_depth_limit() {
        let m = tree();
        let order = ordered_forest(&m, None, 1);
        let keys: Vec<&str> = order.iter().map(|(k, _)| k.as_str()).collect();
        // depth 1 means root + its direct children, not grandchildren (a1)
        assert_eq!(keys, vec!["root", "a", "b"]);
    }

    #[test]
    fn forest_from_specific_key() {
        let m = tree();
        let order = ordered_forest(&m, Some("a"), 10);
        let keys: Vec<&str> = order.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["a", "a1"]);
    }

    #[test]
    fn orphan_parent_treated_as_root() {
        // universe whose parent_key points outside the set
        let mut m = BTreeMap::new();
        m.insert("x".to_string(), d("x", Some("missing"), &[]));
        let order = ordered_forest(&m, None, 10);
        let keys: Vec<&str> = order.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["x"]);
    }

    #[test]
    fn md_block_counts_layer_last() {
        let mut dd = d("u", None, &[]);
        dd.tasks_by_status.insert("todo".to_string(), 3);
        dd.tasks_by_status.insert("done".to_string(), 1);
        dd.content_types = vec!["note".to_string(), "task".to_string()];
        let block = render_md_block(&dd, 0);
        // models appear before counts; counts line is last.
        let models_pos = block.find("models:").unwrap();
        let counts_pos = block.find("counts:").unwrap();
        assert!(models_pos < counts_pos);
        assert!(block.trim_end().ends_with("[done=1 todo=3]"));
    }
}
