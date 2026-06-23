use super::super::*;

// -----------------------------------------------------------------------
// Helper unit tests (no I/O)
// -----------------------------------------------------------------------

#[test]
fn test_slugify() {
    assert_eq!(slugify("Hello World!"), "hello-world");
    assert_eq!(slugify("Article Title"), "article-title");
    assert_eq!(slugify("  spaces  "), "spaces");
    assert_eq!(slugify("CO v1.0 Release"), "co-v10-release");
}

#[test]
fn test_patch_frontmatter_replace() {
    let fm = serde_json::json!({"tags": ["old"], "title": "Test"});
    let result = patch_frontmatter(fm, "tags", r#"["new","updated"]"#, "replace");
    let tags = result["tags"].as_array().unwrap();
    assert!(tags.contains(&serde_json::json!("new")));
    assert!(!tags.contains(&serde_json::json!("old")));
}

#[test]
fn test_patch_frontmatter_append() {
    let fm = serde_json::json!({"tags": ["existing"]});
    let result = patch_frontmatter(fm, "tags", r#""new-tag""#, "append");
    let tags = result["tags"].as_array().unwrap();
    assert!(tags.contains(&serde_json::json!("existing")));
    assert!(tags.contains(&serde_json::json!("new-tag")));
}

#[test]
fn test_extract_matches() {
    let text = "Hello vault world, vault is great";
    let matches = extract_matches(text, "vault", 10);
    assert_eq!(matches.len(), 2);
}

#[test]
fn test_patch_heading_replace() {
    let body = "## Introduction\n\nOld text.\n\n## Other\n\nKeep this.";
    let result = patch_heading(body, "## Introduction", "New text.", "replace");
    assert!(result.contains("New text."), "Got: {result}");
    assert!(!result.contains("Old text."), "Got: {result}");
    assert!(result.contains("## Other"), "Got: {result}");
}

#[test]
fn test_patch_block_replace() {
    let body = "Some paragraph with block ref. ^myblock\n\nOther content.";
    let result = patch_block(body, "^myblock", "Replaced paragraph.", "replace");
    assert!(
        result.contains("Replaced paragraph. ^myblock"),
        "Got: {result}"
    );
    assert!(!result.contains("Some paragraph"), "Got: {result}");
}

// -----------------------------------------------------------------------
// CO-474 (F3): vault path-traversal guard
// -----------------------------------------------------------------------

#[test]
fn test_is_safe_vault_path_accepts_normal_paths() {
    assert!(is_safe_vault_path("notes/hello.md"));
    assert!(is_safe_vault_path("a/b/c/deep.md"));
    assert!(is_safe_vault_path("file.md"));
    assert!(is_safe_vault_path("_universe.yaml"));
    // A literal ".." substring inside a segment name is fine; only a `..`
    // *segment* is a traversal.
    assert!(is_safe_vault_path("my..notes/file.md"));
}

#[test]
fn test_is_safe_vault_path_rejects_traversal_and_absolute() {
    assert!(!is_safe_vault_path(""));
    assert!(!is_safe_vault_path(".."));
    assert!(!is_safe_vault_path("../secret"));
    assert!(!is_safe_vault_path("notes/../../etc/passwd"));
    assert!(!is_safe_vault_path("a/b/../../../c"));
    assert!(!is_safe_vault_path("/etc/passwd")); // absolute
    assert!(!is_safe_vault_path("\\windows\\system")); // backslash absolute
    assert!(!is_safe_vault_path("..\\..\\secret")); // backslash traversal
    assert!(!is_safe_vault_path("C:/Windows")); // drive letter
    assert!(!is_safe_vault_path("file\0.md")); // NUL byte
}
