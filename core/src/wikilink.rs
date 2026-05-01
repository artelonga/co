//! Wikilink parsing for CO-74 typed FK resolution.
//!
//! Parses `[[target]]` and `[[target|alias]]` notation from frontmatter field
//! values and body text.  Typed FK fields declared in the manifest (`ref`,
//! `ref_list`) are resolved via these utilities at entry write time; untyped
//! fields are left as plain text.

/// Strip `[[...]]` wikilink notation, returning just the target path.
///
/// Handles both `[[path]]` and `[[path|alias]]` forms.  If the input is not a
/// wikilink, the trimmed original string is returned unchanged.
pub fn resolve_ref_value(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(inner) = trimmed
        .strip_prefix("[[")
        .and_then(|t| t.strip_suffix("]]"))
    {
        let target = inner.split('|').next().unwrap_or(inner).trim();
        if target.is_empty() { trimmed } else { target }
    } else {
        trimmed
    }
}

/// Extract all wikilink targets from a free-text string.
///
/// Returns resolved paths (without `[[`/`]]` wrappers and without alias part).
pub fn extract_wikilinks(text: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        if let Some(end) = rest.find("]]") {
            let inner = &rest[..end];
            let target = inner.split('|').next().unwrap_or(inner).trim();
            if !target.is_empty() {
                results.push(target.to_string());
            }
            rest = &rest[end + 2..];
        } else {
            break;
        }
    }
    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_plain_wikilink() {
        assert_eq!(resolve_ref_value("[[pessoas/yuri.md]]"), "pessoas/yuri.md");
    }

    #[test]
    fn test_resolve_wikilink_with_alias() {
        assert_eq!(
            resolve_ref_value("[[pessoas/yuri.md|Yuri Figueiredo]]"),
            "pessoas/yuri.md"
        );
    }

    #[test]
    fn test_resolve_plain_path_passthrough() {
        assert_eq!(resolve_ref_value("pessoas/yuri.md"), "pessoas/yuri.md");
    }

    #[test]
    fn test_resolve_trimming() {
        assert_eq!(
            resolve_ref_value("  [[pessoas/yuri.md]]  "),
            "pessoas/yuri.md"
        );
    }

    #[test]
    fn test_resolve_empty_wikilink_returns_original() {
        assert_eq!(resolve_ref_value("[[]]"), "[[]]");
    }

    #[test]
    fn test_extract_wikilinks_basic() {
        let links = extract_wikilinks("See [[foo]] and also [[bar]].");
        assert_eq!(links, vec!["foo", "bar"]);
    }

    #[test]
    fn test_extract_wikilinks_with_alias() {
        let links = extract_wikilinks("Reference [[path/to/note|Note Title]] here.");
        assert_eq!(links, vec!["path/to/note"]);
    }

    #[test]
    fn test_extract_wikilinks_empty_text() {
        assert!(extract_wikilinks("no links here").is_empty());
    }

    #[test]
    fn test_extract_wikilinks_unclosed() {
        let links = extract_wikilinks("[[open but no close");
        assert!(links.is_empty(), "unclosed wikilink must be ignored");
    }

    #[test]
    fn test_extract_multiple_wikilinks() {
        let links = extract_wikilinks("[[a]] mid [[b|alias]] end [[c]]");
        assert_eq!(links, vec!["a", "b", "c"]);
    }
}
