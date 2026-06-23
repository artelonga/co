//! CO-470 — block model over markdown (Phase 1).
//!
//! A [`Block`] is a node in an entry's content tree, parsed *from* the canonical
//! markdown body and serialized back *to* markdown. Markdown stays canonical
//! (the raw-markdown body inside the CO-86 `.co` envelope); this module never
//! sees ciphertext and adds no wire format — see `work/co/CO-470.md`.
//!
//! Phase 1 scope: standard CommonMark/GFM block types with inline content kept
//! as a markdown string, plus a [`BlockType::Raw`] passthrough that preserves
//! any unmodeled construct (tables, HTML, …) verbatim so round-trips never lose
//! data. Extended blocks (callout/toggle/columns) and rich-text annotation spans
//! arrive in Phase 2.
//!
//! Invariant (tested): parsing is idempotent through a serialize round-trip —
//! `parse(serialize(parse(md))) == parse(md)` — i.e. the tree converges after a
//! single normalization. Already-canonical markdown serializes back byte-for-byte.
//!
//! Phase 2 (CO-473) adds the CO-native *extended* container blocks — `callout`,
//! `toggle`, `columns`/`column`, `child_page` — using the portable
//! **fenced-directive** syntax (`:::name {attrs}` … `:::`, Pandoc /
//! `remark-directive` style). The `markdown` crate does not model these, so a
//! focused container pre-parse pass splits `:::` regions out of the body and
//! recurses [`parse_blocks`] on the inner content; the serializer re-emits them.
//! Unknown `:::name` directives degrade to [`BlockType::Raw`] so plain markdown
//! tools that don't understand CO directives still see the inner content.
//!
//! Phase 2 also adds [`assign_ids`]: a deterministic id helper so an editor can
//! address blocks for the `PATCH /blocks` write path. Ids serialize into the
//! directive `{id=…}` slot for extended blocks and an unobtrusive `{#blk_…}`
//! trailer for standard blocks — emitted **only** when an id is present, so
//! unreferenced prose stays clean.

use markdown::mdast::{self, Node};
use markdown::{ParseOptions, to_mdast};
use serde::{Deserialize, Serialize};

/// The kind of a [`Block`]. Phase 1 covers the standard CommonMark/GFM set; any
/// construct not modeled here is preserved verbatim as [`BlockType::Raw`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    Paragraph,
    /// `attrs.level` = 1..=6.
    Heading,
    BulletedListItem,
    NumberedListItem,
    /// `attrs.checked` = bool.
    ToDo,
    Quote,
    /// `attrs.lang` = optional language tag; `text` is the verbatim code.
    Code,
    Divider,
    /// CO-473: highlighted aside. Container — inner markdown is `children`.
    /// `attrs.icon` = optional emoji/icon. Directive name `callout`.
    Callout,
    /// CO-473: collapsible disclosure. Container — `children` is the hidden body.
    /// `attrs.summary` = optional summary line. Directive name `toggle`.
    Toggle,
    /// CO-473: multi-column layout. Container of [`BlockType::Column`] children.
    /// Directive name `columns`.
    Columns,
    /// CO-473: a single column inside [`BlockType::Columns`]. Container — inner
    /// markdown is `children`. Directive name `column`.
    Column,
    /// CO-473: an inline reference to a nested page. Container — `children`
    /// holds the nested content. `attrs.title` = optional page title. Directive
    /// name `child_page`.
    ChildPage,
    /// Verbatim markdown the Phase 1 model does not destructure (tables, HTML,
    /// footnotes, …). `text` holds the exact source; serialization re-emits it.
    Raw,
}

impl BlockType {
    /// The fenced-directive name for an extended (container) block, or `None`
    /// for standard CommonMark/GFM blocks that need no directive syntax.
    fn directive_name(&self) -> Option<&'static str> {
        match self {
            BlockType::Callout => Some("callout"),
            BlockType::Toggle => Some("toggle"),
            BlockType::Columns => Some("columns"),
            BlockType::Column => Some("column"),
            BlockType::ChildPage => Some("child_page"),
            _ => None,
        }
    }

    /// Map a directive name back to its extended [`BlockType`]. Unknown names
    /// return `None` so the caller can degrade to [`BlockType::Raw`].
    fn from_directive_name(name: &str) -> Option<BlockType> {
        match name {
            "callout" => Some(BlockType::Callout),
            "toggle" => Some(BlockType::Toggle),
            "columns" => Some(BlockType::Columns),
            "column" => Some(BlockType::Column),
            "child_page" => Some(BlockType::ChildPage),
            _ => None,
        }
    }
}

/// A node in an entry's content tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Stable address, assigned only when something references the block
    /// (comments, deep links). `None` for ordinary prose — IDs never pollute
    /// unreferenced content. Phase 1 always parses `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub block_type: BlockType,
    /// Type-specific attributes (heading level, code language, todo checked…).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub attrs: serde_json::Map<String, serde_json::Value>,
    /// Inline content as a markdown string (leaf blocks). `None` for containers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Child blocks (list nesting, quote contents).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Block>,
}

impl Block {
    fn leaf(block_type: BlockType, text: Option<String>) -> Self {
        Block {
            id: None,
            block_type,
            attrs: serde_json::Map::new(),
            text,
            children: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Parse: markdown → blocks
// ---------------------------------------------------------------------------

/// Parse a markdown body into a flat-ish block tree. Frontmatter is *not*
/// handled here — pass the body only (the caller strips frontmatter via
/// `co::frontmatter`).
///
/// CO-473: a fenced-directive pre-pass splits out `:::name {attrs}` … `:::`
/// container regions (callout/toggle/columns/child_page) before the markdown
/// crate sees them — the `markdown` crate does not model directives — and
/// recurses into their inner content. Everything between directives is ordinary
/// markdown parsed by [`map_nodes`].
pub fn parse_blocks(markdown_body: &str) -> Vec<Block> {
    parse_with_directives(markdown_body)
}

/// Parse markdown standard nodes only (no directive pre-pass). Used for the
/// inner-content recursion after a directive region is already isolated, and as
/// the leaf of the directive splitter.
fn parse_standard(markdown_body: &str) -> Vec<Block> {
    // GFM so task lists + tables are recognized. The parser is total for our
    // purposes: on the (practically unreachable) error path we fall back to a
    // single Raw block holding the whole body, which still round-trips.
    let opts = ParseOptions::gfm();
    match to_mdast(markdown_body, &opts) {
        Ok(Node::Root(root)) => map_nodes(&root.children, markdown_body),
        Ok(other) => map_nodes(std::slice::from_ref(&other), markdown_body),
        Err(_) => vec![Block::leaf(BlockType::Raw, Some(markdown_body.to_string()))],
    }
}

// ---------------------------------------------------------------------------
// CO-473: fenced-directive container pre-parse
// ---------------------------------------------------------------------------

/// An opening directive fence: `:::name {attrs}`. We accept three-or-more
/// colons and optional surrounding whitespace so `:::columns` and `::: column`
/// both parse (matching the documented example).
struct OpenFence {
    name: String,
    attrs: serde_json::Map<String, serde_json::Value>,
    id: Option<String>,
}

/// Recognize a line as an opening directive fence. Returns `None` for a bare
/// `:::` (treated as a *close*) or any non-fence line.
fn parse_open_fence(line: &str) -> Option<OpenFence> {
    let t = line.trim();
    let rest = t.strip_prefix(":::")?;
    // Allow extra leading colons (`::::name`) — collapse them.
    let rest = rest.trim_start_matches(':');
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None; // bare ":::" is a close, not an open.
    }
    // name = leading ident; remainder may carry a `{...}` attr block.
    let name_end = rest
        .find(|c: char| c.is_whitespace() || c == '{')
        .unwrap_or(rest.len());
    let name = rest[..name_end].to_string();
    if name.is_empty() {
        return None;
    }
    let attr_src = rest[name_end..].trim();
    let (attrs, id) = parse_attr_block(attr_src);
    Some(OpenFence { name, attrs, id })
}

/// True for a bare closing fence line (`:::`).
fn is_close_fence(line: &str) -> bool {
    let t = line.trim();
    t.starts_with(":::") && t.trim_start_matches(':').trim().is_empty()
}

/// Parse a `{key="value" key2=value2 #blk_id id=blk_x}` attribute block into a
/// JSON map plus the extracted block id (from `id=` or `#blk_…`). Tolerant: a
/// missing/blank block yields an empty map.
fn parse_attr_block(src: &str) -> (serde_json::Map<String, serde_json::Value>, Option<String>) {
    let mut attrs = serde_json::Map::new();
    let mut id = None;
    let inner = src
        .trim()
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .unwrap_or("");
    for tok in tokenize_attrs(inner) {
        if let Some(anchor) = tok.strip_prefix('#') {
            id = Some(anchor.to_string());
            continue;
        }
        if let Some((k, v)) = tok.split_once('=') {
            let v = v.trim().trim_matches('"').to_string();
            if k.trim() == "id" {
                id = Some(v);
            } else {
                attrs.insert(k.trim().to_string(), serde_json::Value::from(v));
            }
        }
    }
    (attrs, id)
}

/// Split an attribute block's interior into tokens, honoring double-quoted
/// values so `summary="a b"` stays one token.
fn tokenize_attrs(inner: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for c in inner.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    toks.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

/// Parse a body that may contain directive containers. Plain-markdown runs are
/// handed to [`parse_standard`]; each top-level `:::name … :::` region becomes
/// one extended block whose inner content is parsed recursively.
fn parse_with_directives(body: &str) -> Vec<Block> {
    let lines: Vec<&str> = body.lines().collect();
    let mut out: Vec<Block> = Vec::new();
    let mut plain_start = 0usize;
    let mut i = 0usize;

    let flush_plain = |out: &mut Vec<Block>, slice: &[&str]| {
        let text = slice.join("\n");
        if !text.trim().is_empty() {
            out.extend(parse_standard(&text));
        }
    };

    while i < lines.len() {
        if let Some(open) = parse_open_fence(lines[i]) {
            // Close the preceding plain run.
            flush_plain(&mut out, &lines[plain_start..i]);
            // Find the matching close, tracking nested opens.
            let mut depth = 1usize;
            let mut j = i + 1;
            while j < lines.len() {
                if parse_open_fence(lines[j]).is_some() {
                    depth += 1;
                } else if is_close_fence(lines[j]) {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            let inner = if j > i + 1 {
                lines[i + 1..j].join("\n")
            } else {
                String::new()
            };
            match BlockType::from_directive_name(&open.name) {
                Some(bt) => {
                    let mut b = Block::leaf(bt, None);
                    b.attrs = open.attrs;
                    b.id = open.id;
                    b.children = parse_with_directives(&inner);
                    out.push(b);
                }
                None => {
                    // Unknown directive — degrade to Raw, preserving the whole
                    // region verbatim so plain markdown tools are unaffected.
                    let end = j.min(lines.len().saturating_sub(1));
                    let verbatim = lines[i..=end.max(i)].join("\n");
                    out.push(Block::leaf(BlockType::Raw, Some(verbatim)));
                }
            }
            i = if j < lines.len() { j + 1 } else { j };
            plain_start = i;
        } else {
            i += 1;
        }
    }
    flush_plain(&mut out, &lines[plain_start..]);
    out
}

fn map_nodes(nodes: &[Node], source: &str) -> Vec<Block> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            // A list expands into one sibling block per item.
            Node::List(list) => out.extend(list_items(list, source)),
            other => {
                if let Some(b) = map_node(other, source) {
                    out.push(b);
                }
            }
        }
    }
    out
}

/// CO-473: strip a trailing `{#blk_…}` id anchor from a standard block's inline
/// text. Returns the cleaned text and the extracted id (if any). Only a single
/// trailing anchor is recognized so prose containing `{#…}` elsewhere is intact.
fn split_id_trailer(text: &str) -> (String, Option<String>) {
    let trimmed = text.trim_end();
    if let Some(rest) = trimmed.strip_suffix('}')
        && let Some(open) = rest.rfind("{#")
    {
        let id = &rest[open + 2..];
        if !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            let head = rest[..open].trim_end().to_string();
            return (head, Some(id.to_string()));
        }
    }
    (text.to_string(), None)
}

fn map_node(node: &Node, source: &str) -> Option<Block> {
    match node {
        Node::Paragraph(p) => {
            let (text, id) = split_id_trailer(&inline_to_md(&p.children, source));
            let mut b = Block::leaf(BlockType::Paragraph, Some(text));
            b.id = id;
            Some(b)
        }
        Node::Heading(h) => {
            let (text, id) = split_id_trailer(&inline_to_md(&h.children, source));
            let mut b = Block::leaf(BlockType::Heading, Some(text));
            b.id = id;
            b.attrs
                .insert("level".into(), serde_json::Value::from(h.depth.clamp(1, 6)));
            Some(b)
        }
        Node::Code(c) => {
            let mut b = Block::leaf(BlockType::Code, Some(c.value.clone()));
            if let Some(lang) = &c.lang {
                b.attrs
                    .insert("lang".into(), serde_json::Value::from(lang.clone()));
            }
            Some(b)
        }
        Node::ThematicBreak(_) => Some(Block::leaf(BlockType::Divider, None)),
        Node::Blockquote(q) => {
            let mut b = Block::leaf(BlockType::Quote, None);
            b.children = map_nodes(&q.children, source);
            Some(b)
        }
        // Lists are expanded by `map_nodes` (one block per item), never here.
        Node::List(_) => None,
        // Unmodeled (tables, HTML, definitions, …): preserve verbatim.
        other => Some(Block::leaf(
            BlockType::Raw,
            Some(node_source(other, source)),
        )),
    }
}

/// Expand a list's items into individual blocks (with nested children).
fn list_items(list: &mdast::List, source: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut counter = list.start.unwrap_or(1);
    for child in &list.children {
        if let Node::ListItem(item) = child {
            out.push(map_list_item(item, list.ordered, &mut counter, source));
            counter += 1;
        }
    }
    out
}

fn map_list_item(item: &mdast::ListItem, ordered: bool, counter: &mut u32, source: &str) -> Block {
    // First paragraph (if any) becomes the item's inline text; remaining block
    // children (incl. nested lists) become `children`.
    let mut text: Option<String> = None;
    let mut children: Vec<Block> = Vec::new();
    for (i, c) in item.children.iter().enumerate() {
        match c {
            Node::Paragraph(p) if i == 0 => text = Some(inline_to_md(&p.children, source)),
            Node::List(nested) => children.extend(list_items(nested, source)),
            other => {
                if let Some(b) = map_node(other, source) {
                    children.push(b);
                }
            }
        }
    }
    let block_type = match item.checked {
        Some(_) => BlockType::ToDo,
        None if ordered => BlockType::NumberedListItem,
        None => BlockType::BulletedListItem,
    };
    let (text, id) = match text {
        Some(t) => {
            let (clean, id) = split_id_trailer(&t);
            (Some(clean), id)
        }
        None => (None, None),
    };
    let mut b = Block::leaf(block_type, text);
    b.id = id;
    b.children = children;
    if let Some(checked) = item.checked {
        b.attrs
            .insert("checked".into(), serde_json::Value::from(checked));
    }
    if ordered {
        b.attrs
            .insert("number".into(), serde_json::Value::from(*counter));
    }
    b
}

/// Slice the original source for a node, falling back to a best-effort render.
fn node_source(node: &Node, source: &str) -> String {
    if let Some(pos) = node.position() {
        let (s, e) = (pos.start.offset, pos.end.offset);
        if s <= e && e <= source.len() {
            return source[s..e].to_string();
        }
    }
    // No position (shouldn't happen for block nodes from to_mdast): empty.
    String::new()
}

/// Serialize inline children back to a markdown string. Uses the source slice
/// (verbatim, lossless) when position info is available; otherwise reconstructs
/// from the common inline node types.
fn inline_to_md(children: &[Node], source: &str) -> String {
    // The parent's children collectively span a contiguous source range; slicing
    // from the first child's start to the last child's end reproduces the
    // original inline markdown verbatim (preserving `*`, `_`, links, etc.).
    if let (Some(first), Some(last)) = (children.first(), children.last())
        && let (Some(fp), Some(lp)) = (first.position(), last.position())
    {
        let (s, e) = (fp.start.offset, lp.end.offset);
        if s <= e && e <= source.len() {
            return source[s..e].to_string();
        }
    }
    // Fallback: concatenate text values.
    children.iter().map(node_text).collect()
}

fn node_text(node: &Node) -> String {
    match node {
        Node::Text(t) => t.value.clone(),
        Node::InlineCode(c) => format!("`{}`", c.value),
        _ => node
            .children()
            .map(|c| c.iter().map(node_text).collect::<String>())
            .unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Serialize: blocks → markdown
// ---------------------------------------------------------------------------

/// Serialize a block tree back to a canonical markdown body.
pub fn serialize_blocks(blocks: &[Block]) -> String {
    let mut out = String::new();
    serialize_into(blocks, &mut out, 0);
    // Normalize: single trailing newline, no leading blank.
    let trimmed = out.trim_matches('\n');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

fn is_list_item(t: &BlockType) -> bool {
    matches!(
        t,
        BlockType::BulletedListItem | BlockType::NumberedListItem | BlockType::ToDo
    )
}

fn serialize_into(blocks: &[Block], out: &mut String, indent: usize) {
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            // Consecutive list items stay tight (no blank line between them);
            // every other block boundary gets a blank line (CommonMark).
            let prev = &blocks[i - 1].block_type;
            let tight = is_list_item(prev) && is_list_item(&b.block_type);
            if !tight {
                out.push('\n');
            }
        }
        serialize_block(b, out, indent);
    }
}

fn serialize_block(b: &Block, out: &mut String, indent: usize) {
    let pad = "  ".repeat(indent);
    match b.block_type {
        BlockType::Paragraph => {
            out.push_str(&pad);
            out.push_str(b.text.as_deref().unwrap_or(""));
            push_id_trailer(b.id.as_deref(), out);
            out.push('\n');
        }
        BlockType::Heading => {
            let level = b
                .attrs
                .get("level")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            out.push_str(&pad);
            out.push_str(&"#".repeat(level));
            out.push(' ');
            out.push_str(b.text.as_deref().unwrap_or(""));
            push_id_trailer(b.id.as_deref(), out);
            out.push('\n');
        }
        BlockType::Code => {
            let lang = b.attrs.get("lang").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&pad);
            out.push_str("```");
            out.push_str(lang);
            out.push('\n');
            out.push_str(b.text.as_deref().unwrap_or(""));
            out.push('\n');
            out.push_str(&pad);
            out.push_str("```\n");
        }
        BlockType::Divider => {
            out.push_str(&pad);
            out.push_str("---\n");
        }
        BlockType::Quote => {
            // Render children, then prefix each line with "> ".
            let mut inner = String::new();
            serialize_into(&b.children, &mut inner, 0);
            for line in inner.trim_end_matches('\n').split('\n') {
                out.push_str(&pad);
                if line.is_empty() {
                    out.push('>');
                } else {
                    out.push_str("> ");
                    out.push_str(line);
                }
                out.push('\n');
            }
        }
        BlockType::BulletedListItem | BlockType::NumberedListItem | BlockType::ToDo => {
            out.push_str(&pad);
            let marker = match b.block_type {
                BlockType::NumberedListItem => {
                    let n = b.attrs.get("number").and_then(|v| v.as_u64()).unwrap_or(1);
                    format!("{n}. ")
                }
                BlockType::ToDo => {
                    let checked = b
                        .attrs
                        .get("checked")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    format!("- [{}] ", if checked { "x" } else { " " })
                }
                _ => "- ".to_string(),
            };
            out.push_str(&marker);
            out.push_str(b.text.as_deref().unwrap_or(""));
            push_id_trailer(b.id.as_deref(), out);
            out.push('\n');
            if !b.children.is_empty() {
                serialize_into(&b.children, out, indent + 1);
            }
        }
        BlockType::Callout | BlockType::Toggle | BlockType::Column | BlockType::ChildPage => {
            serialize_directive(b, out, indent);
        }
        BlockType::Columns => {
            serialize_directive(b, out, indent);
        }
        BlockType::Raw => {
            out.push_str(b.text.as_deref().unwrap_or(""));
            out.push('\n');
        }
    }
}

/// CO-473: serialize an extended container block as a fenced directive.
/// `:::name {attrs id=…}` / inner / `:::`. Inner content (children) is
/// serialized recursively, so nested directives (a `column` inside `columns`)
/// round-trip.
fn serialize_directive(b: &Block, out: &mut String, indent: usize) {
    let pad = "  ".repeat(indent);
    let name = b
        .block_type
        .directive_name()
        .expect("serialize_directive called on a non-directive block");
    out.push_str(&pad);
    out.push_str(":::");
    out.push_str(name);
    let attr = format_attr_block(&b.attrs, b.id.as_deref());
    if !attr.is_empty() {
        out.push(' ');
        out.push_str(&attr);
    }
    out.push('\n');
    // Inner content. Directives are always parsed at the body's column 0, so we
    // serialize children at indent 0 inside the fence (the fence itself carries
    // the nesting), keeping the round-trip stable.
    let mut inner = String::new();
    serialize_into(&b.children, &mut inner, 0);
    let inner = inner.trim_matches('\n');
    if !inner.is_empty() {
        out.push_str(inner);
        out.push('\n');
    }
    out.push_str(&pad);
    out.push_str(":::\n");
}

/// CO-473: append an unobtrusive ` {#blk_…}` id anchor to a standard block's
/// serialized line, but only when the block carries an id. Unreferenced prose
/// gets no anchor, so plain bodies stay clean.
fn push_id_trailer(id: Option<&str>, out: &mut String) {
    if let Some(id) = id {
        out.push_str(" {#");
        out.push_str(id);
        out.push('}');
    }
}

/// Render an attribute map (+optional id) as `{key="value" … id=blk_x}`.
/// Returns "" when there is nothing to emit.
fn format_attr_block(
    attrs: &serde_json::Map<String, serde_json::Value>,
    id: Option<&str>,
) -> String {
    if attrs.is_empty() && id.is_none() {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in attrs {
        let val = match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        parts.push(format!("{k}=\"{val}\""));
    }
    if let Some(id) = id {
        parts.push(format!("id={id}"));
    }
    format!("{{{}}}", parts.join(" "))
}

// ---------------------------------------------------------------------------
// CO-473: deterministic block-id assignment + id-addressed PATCH ops
// ---------------------------------------------------------------------------

/// Assign deterministic, stable ids to every block that does not already carry
/// one, recursing into children. The scheme is a depth-first ordinal path
/// (`blk_1`, `blk_1_1`, `blk_2`, …) so the same tree always yields the same ids
/// — an editor can address a block by id for the `PATCH /blocks` write path
/// without a database round-trip. Blocks that already have an id keep it.
pub fn assign_ids(blocks: &mut [Block]) {
    assign_ids_inner(blocks, "blk");
}

fn assign_ids_inner(blocks: &mut [Block], prefix: &str) {
    for (i, b) in blocks.iter_mut().enumerate() {
        let path = format!("{prefix}_{}", i + 1);
        if b.id.is_none() {
            b.id = Some(path.clone());
        }
        // Recurse using the assigned/own id as the child prefix so nested ids
        // stay stable and unique even when a parent already had an id.
        let child_prefix = b.id.clone().unwrap_or(path);
        assign_ids_inner(&mut b.children, &child_prefix);
    }
}

/// An id-addressed edit on a block tree (the `PATCH /blocks` op vocabulary).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BlockOp {
    /// Replace the block addressed by `id` with `block` (the new block keeps the
    /// target's id unless it carries its own).
    Replace { id: String, block: Block },
    /// Insert `block` immediately after the block addressed by `id`.
    InsertAfter { id: String, block: Block },
    /// Delete the block addressed by `id`.
    Delete { id: String },
    /// Move the block addressed by `id` to immediately after the block
    /// addressed by `after`.
    Move { id: String, after: String },
}

/// Error applying a [`BlockOp`] — the addressed id was not found in the tree.
#[derive(Debug, PartialEq, Eq)]
pub struct OpError {
    pub message: String,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for OpError {}

/// Apply a batch of id-addressed [`BlockOp`]s to a tree in order. Ids are
/// assumed already assigned (call [`assign_ids`] first). Operates recursively so
/// blocks nested inside containers (callout/column/…) are addressable. An op
/// that addresses a missing id is a hard error — the caller maps it to a 4xx.
pub fn apply_ops(blocks: &mut Vec<Block>, ops: &[BlockOp]) -> Result<(), OpError> {
    for op in ops {
        match op {
            BlockOp::Replace { id, block } => {
                let target = find_mut(blocks, id).ok_or_else(|| OpError {
                    message: format!("block id '{id}' not found"),
                })?;
                let mut new_block = block.clone();
                if new_block.id.is_none() {
                    new_block.id = Some(id.clone());
                }
                *target = new_block;
            }
            BlockOp::InsertAfter { id, block } => {
                if !insert_after(blocks, id, block.clone()) {
                    return Err(OpError {
                        message: format!("block id '{id}' not found"),
                    });
                }
            }
            BlockOp::Delete { id } => {
                if remove_by_id(blocks, id).is_none() {
                    return Err(OpError {
                        message: format!("block id '{id}' not found"),
                    });
                }
            }
            BlockOp::Move { id, after } => {
                let moved = remove_by_id(blocks, id).ok_or_else(|| OpError {
                    message: format!("block id '{id}' not found"),
                })?;
                if !insert_after(blocks, after, moved) {
                    return Err(OpError {
                        message: format!("target block id '{after}' not found"),
                    });
                }
            }
        }
    }
    Ok(())
}

/// Find a mutable reference to the block with `id`, recursing into children.
fn find_mut<'a>(blocks: &'a mut [Block], id: &str) -> Option<&'a mut Block> {
    for b in blocks.iter_mut() {
        if b.id.as_deref() == Some(id) {
            return Some(b);
        }
        if let Some(found) = find_mut(&mut b.children, id) {
            return Some(found);
        }
    }
    None
}

/// Remove the block with `id` (searching the whole tree) and return it.
fn remove_by_id(blocks: &mut Vec<Block>, id: &str) -> Option<Block> {
    if let Some(pos) = blocks.iter().position(|b| b.id.as_deref() == Some(id)) {
        return Some(blocks.remove(pos));
    }
    for b in blocks.iter_mut() {
        if let Some(found) = remove_by_id(&mut b.children, id) {
            return Some(found);
        }
    }
    None
}

/// Insert `block` immediately after the block with `id` (searching the whole
/// tree). Returns `false` if `id` was not found.
fn insert_after(blocks: &mut Vec<Block>, id: &str, block: Block) -> bool {
    if let Some(pos) = blocks.iter().position(|b| b.id.as_deref() == Some(id)) {
        blocks.insert(pos + 1, block);
        return true;
    }
    for b in blocks.iter_mut() {
        if insert_after(&mut b.children, id, block.clone()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests;
