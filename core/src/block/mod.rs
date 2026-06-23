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
    /// Verbatim markdown the Phase 1 model does not destructure (tables, HTML,
    /// footnotes, …). `text` holds the exact source; serialization re-emits it.
    Raw,
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
pub fn parse_blocks(markdown_body: &str) -> Vec<Block> {
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

fn map_node(node: &Node, source: &str) -> Option<Block> {
    match node {
        Node::Paragraph(p) => Some(Block::leaf(
            BlockType::Paragraph,
            Some(inline_to_md(&p.children, source)),
        )),
        Node::Heading(h) => {
            let mut b = Block::leaf(BlockType::Heading, Some(inline_to_md(&h.children, source)));
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
    let mut b = Block::leaf(block_type, text);
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
            out.push('\n');
            if !b.children.is_empty() {
                serialize_into(&b.children, out, indent + 1);
            }
        }
        BlockType::Raw => {
            out.push_str(b.text.as_deref().unwrap_or(""));
            out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests;
