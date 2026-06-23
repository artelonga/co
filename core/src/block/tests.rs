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

// ---------------------------------------------------------------------------
// CO-473 — extended blocks (fenced directives), id assignment, PATCH ops
// ---------------------------------------------------------------------------

#[test]
fn parses_callout_directive() {
    let md = ":::callout {icon=\"💡\"}\ninner **markdown**\n:::\n";
    let blocks = parse_blocks(md);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_type, BlockType::Callout);
    assert_eq!(blocks[0].attrs.get("icon").unwrap(), "💡");
    assert_eq!(blocks[0].children.len(), 1);
    assert_eq!(blocks[0].children[0].block_type, BlockType::Paragraph);
    assert_eq!(
        blocks[0].children[0].text.as_deref(),
        Some("inner **markdown**")
    );
}

#[test]
fn parses_toggle_with_summary_and_id() {
    let md = ":::toggle {summary=\"Details\" id=blk_t}\nhidden\n:::\n";
    let blocks = parse_blocks(md);
    assert_eq!(blocks[0].block_type, BlockType::Toggle);
    assert_eq!(blocks[0].attrs.get("summary").unwrap(), "Details");
    assert_eq!(blocks[0].id.as_deref(), Some("blk_t"));
}

#[test]
fn parses_nested_columns() {
    let md = ":::columns\n::: column\nleft\n:::\n::: column\nright\n:::\n:::\n";
    let blocks = parse_blocks(md);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_type, BlockType::Columns);
    assert_eq!(blocks[0].children.len(), 2);
    assert_eq!(blocks[0].children[0].block_type, BlockType::Column);
    assert_eq!(blocks[0].children[1].block_type, BlockType::Column);
    assert_eq!(
        blocks[0].children[0].children[0].text.as_deref(),
        Some("left")
    );
    assert_eq!(
        blocks[0].children[1].children[0].text.as_deref(),
        Some("right")
    );
}

#[test]
fn unknown_directive_degrades_to_raw() {
    // A non-CO directive a plain markdown tool might use is preserved verbatim,
    // not lost — graceful degradation.
    let md = ":::aside {note=\"x\"}\nsome text\n:::\n";
    let blocks = parse_blocks(md);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].block_type, BlockType::Raw);
    assert!(blocks[0].text.as_deref().unwrap().contains(":::aside"));
    assert!(blocks[0].text.as_deref().unwrap().contains("some text"));
}

#[test]
fn extended_blocks_round_trip_idempotent() {
    let inputs = [
        ":::callout {icon=\"💡\"}\ninner **markdown**\n:::\n",
        ":::toggle {summary=\"Details\"}\nhidden\n:::\n",
        ":::columns\n::: column\nleft\n:::\n::: column\nright\n:::\n:::\n",
        ":::child_page {title=\"Sub\"}\n# Sub page\n\nbody\n:::\n",
        // Mixed: prose before/after a directive.
        "intro para\n\n:::callout {icon=\"⚠️\"}\nwatch out\n:::\n\noutro para\n",
    ];
    for md in inputs {
        let once = parse_blocks(md);
        let twice = parse_blocks(&serialize_blocks(&once));
        assert_eq!(once, twice, "not idempotent for input: {md:?}");
    }
}

#[test]
fn assign_ids_is_deterministic_and_stable() {
    let md = "# T\n\npara\n\n:::callout {icon=\"i\"}\ninner\n:::\n";
    let mut a = parse_blocks(md);
    let mut b = parse_blocks(md);
    assign_ids(&mut a);
    assign_ids(&mut b);
    assert_eq!(a, b, "id assignment must be deterministic");
    assert!(a.iter().all(|blk| blk.id.is_some()));
    // Ids survive a serialize→parse cycle.
    let reparsed = parse_blocks(&serialize_blocks(&a));
    let ids_before: Vec<_> = a.iter().map(|blk| blk.id.clone()).collect();
    let ids_after: Vec<_> = reparsed.iter().map(|blk| blk.id.clone()).collect();
    assert_eq!(ids_before, ids_after, "ids must round-trip");
}

#[test]
fn assign_ids_preserves_existing() {
    let mut blocks = parse_blocks(":::toggle {id=blk_keep}\nx\n:::\n");
    assign_ids(&mut blocks);
    assert_eq!(blocks[0].id.as_deref(), Some("blk_keep"));
}

#[test]
fn ids_absent_for_unreferenced_prose() {
    // Plain serialize never injects anchors when ids are None.
    let md = "# T\n\npara\n";
    let serialized = serialize_blocks(&parse_blocks(md));
    assert_eq!(serialized, md);
    assert!(!serialized.contains("{#"));
}

#[test]
fn apply_replace_op() {
    let mut blocks = parse_blocks("para one\n\npara two\n");
    assign_ids(&mut blocks);
    let target_id = blocks[0].id.clone().unwrap();
    let new = Block {
        id: None,
        block_type: BlockType::Paragraph,
        attrs: Default::default(),
        text: Some("replaced".into()),
        children: vec![],
    };
    apply_ops(
        &mut blocks,
        &[BlockOp::Replace {
            id: target_id.clone(),
            block: new,
        }],
    )
    .unwrap();
    assert_eq!(blocks[0].text.as_deref(), Some("replaced"));
    assert_eq!(blocks[0].id.as_deref(), Some(target_id.as_str()));
}

#[test]
fn apply_insert_after_and_delete_and_move() {
    let mut blocks = parse_blocks("a\n\nb\n\nc\n");
    assign_ids(&mut blocks);
    let id_a = blocks[0].id.clone().unwrap();
    let id_c = blocks[2].id.clone().unwrap();
    let inserted = Block {
        id: Some("blk_new".into()),
        block_type: BlockType::Paragraph,
        attrs: Default::default(),
        text: Some("inserted".into()),
        children: vec![],
    };
    apply_ops(
        &mut blocks,
        &[BlockOp::InsertAfter {
            id: id_a.clone(),
            block: inserted,
        }],
    )
    .unwrap();
    assert_eq!(blocks[1].text.as_deref(), Some("inserted"));

    // Move the inserted block after c.
    apply_ops(
        &mut blocks,
        &[BlockOp::Move {
            id: "blk_new".into(),
            after: id_c,
        }],
    )
    .unwrap();
    assert_eq!(blocks.last().unwrap().text.as_deref(), Some("inserted"));

    // Delete a.
    apply_ops(&mut blocks, &[BlockOp::Delete { id: id_a }]).unwrap();
    assert!(blocks.iter().all(|b| b.text.as_deref() != Some("a")));
}

#[test]
fn apply_op_missing_id_errors() {
    let mut blocks = parse_blocks("a\n");
    assign_ids(&mut blocks);
    let err = apply_ops(&mut blocks, &[BlockOp::Delete { id: "nope".into() }]);
    assert!(err.is_err());
}

#[test]
fn ops_address_nested_blocks() {
    let mut blocks = parse_blocks(":::callout {icon=\"i\"}\ninner\n:::\n");
    assign_ids(&mut blocks);
    let inner_id = blocks[0].children[0].id.clone().unwrap();
    apply_ops(&mut blocks, &[BlockOp::Delete { id: inner_id }]).unwrap();
    assert!(blocks[0].children.is_empty());
}
