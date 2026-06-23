//! CO-470 Phase 1 — parse/serialize + round-trip tests.

use super::*;

fn types(blocks: &[Block]) -> Vec<&BlockType> {
    blocks.iter().map(|b| &b.block_type).collect()
}

#[test]
fn parses_standard_block_types() {
    let md = "# Title\n\nA paragraph.\n\n```rust\nlet x = 1;\n```\n\n---\n";
    let blocks = parse_blocks(md);
    assert_eq!(
        types(&blocks),
        vec![
            &BlockType::Heading,
            &BlockType::Paragraph,
            &BlockType::Code,
            &BlockType::Divider,
        ]
    );
    assert_eq!(blocks[0].attrs.get("level").unwrap(), 1);
    assert_eq!(blocks[0].text.as_deref(), Some("Title"));
    assert_eq!(blocks[2].attrs.get("lang").unwrap(), "rust");
    assert_eq!(blocks[2].text.as_deref(), Some("let x = 1;"));
}

#[test]
fn parses_list_items_with_markers() {
    let md = "- one\n- two\n";
    let blocks = parse_blocks(md);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].block_type, BlockType::BulletedListItem);
    assert_eq!(blocks[0].text.as_deref(), Some("one"));

    let ordered = parse_blocks("1. a\n2. b\n");
    assert_eq!(ordered[0].block_type, BlockType::NumberedListItem);
    assert_eq!(ordered[1].attrs.get("number").unwrap(), 2);

    let todo = parse_blocks("- [ ] todo\n- [x] done\n");
    assert_eq!(todo[0].block_type, BlockType::ToDo);
    assert_eq!(todo[0].attrs.get("checked").unwrap(), false);
    assert_eq!(todo[1].attrs.get("checked").unwrap(), true);
}

#[test]
fn nested_list_becomes_children() {
    let md = "- parent\n  - child\n";
    let blocks = parse_blocks(md);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text.as_deref(), Some("parent"));
    assert_eq!(blocks[0].children.len(), 1);
    assert_eq!(blocks[0].children[0].text.as_deref(), Some("child"));
}

#[test]
fn inline_formatting_preserved_verbatim() {
    // Inline markdown is kept as a string slice from source — *not* normalized.
    let md = "A *bold* and `code` and [link](http://x).\n";
    let blocks = parse_blocks(md);
    assert_eq!(
        blocks[0].text.as_deref(),
        Some("A *bold* and `code` and [link](http://x).")
    );
}

#[test]
fn unmodeled_construct_preserved_as_raw() {
    // A GFM table is not destructured in Phase 1 — kept verbatim as Raw.
    let md = "| a | b |\n| - | - |\n| 1 | 2 |\n";
    let blocks = parse_blocks(md);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_type, BlockType::Raw);
    assert!(blocks[0].text.as_deref().unwrap().contains("| a | b |"));
}

#[test]
fn serialize_round_trips_canonical_markdown() {
    // Already-canonical markdown serializes back byte-for-byte.
    for md in [
        "# Heading\n",
        "A paragraph.\n",
        "- one\n- two\n",
        "1. a\n2. b\n",
        "- [ ] todo\n- [x] done\n",
        "```rust\nlet x = 1;\n```\n",
        "---\n",
    ] {
        let got = serialize_blocks(&parse_blocks(md));
        assert_eq!(got, md, "round-trip changed canonical markdown: {md:?}");
    }
}

#[test]
fn parse_is_idempotent_through_serialize() {
    // The keystone invariant: parse(serialize(parse(md))) == parse(md).
    // The tree converges after a single normalization for arbitrary input.
    let inputs = [
        "# T\n\npara with *em* and `c`\n\n- a\n- b\n\n> quote\n\n```py\nx=1\n```\n",
        "Mixed _underscore_ emphasis and a [[uni::path]] mention.\n",
        "- parent\n  - child\n  - child2\n- sibling\n",
        "| a | b |\n| - | - |\n| 1 | 2 |\n",
        "## H2\n\n1. first\n2. second\n\n---\n\nlast para\n",
    ];
    for md in inputs {
        let once = parse_blocks(md);
        let twice = parse_blocks(&serialize_blocks(&once));
        assert_eq!(once, twice, "not idempotent for input: {md:?}");
    }
}

#[test]
fn quote_round_trips() {
    let md = "> a quote\n";
    let blocks = parse_blocks(md);
    assert_eq!(blocks[0].block_type, BlockType::Quote);
    // Reparse-after-serialize must converge.
    assert_eq!(blocks, parse_blocks(&serialize_blocks(&blocks)));
}

#[test]
fn ids_are_none_for_plain_prose() {
    let blocks = parse_blocks("# T\n\npara\n");
    assert!(blocks.iter().all(|b| b.id.is_none()));
}

#[test]
fn empty_body_yields_no_blocks() {
    assert!(parse_blocks("").is_empty());
    assert_eq!(serialize_blocks(&[]), "");
}
