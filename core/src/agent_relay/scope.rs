//! CO-110 agent-side path scoping.
//!
//! The `co-agent` exposes only paths under its configured `roots`. Every request
//! carries a path *relative* to a root; the agent resolves it with this module,
//! which rejects anything that would escape the root:
//!
//! * absolute paths (`/etc/passwd`),
//! * parent traversal (`../`, including after normalization),
//! * Windows drive / UNC prefixes and backslash separators,
//! * NUL bytes.
//!
//! Resolution is **purely lexical** — it never touches the filesystem, so it is
//! deterministic and testable without disk. A real agent additionally guards
//! against symlink escapes by canonicalizing and re-checking containment after
//! `open` (see the threat model); that check is host-dependent and out of scope
//! for this lexical core.

use std::path::{Component, Path, PathBuf};

/// Why a requested path was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScopeError {
    /// The path was absolute (must be relative to a root).
    #[error("path must be relative, not absolute")]
    Absolute,
    /// The path tried to escape its root via `..`.
    #[error("path escapes its root")]
    Traversal,
    /// The path contained a NUL byte or other rejected byte.
    #[error("path contains an illegal byte")]
    IllegalByte,
    /// The path used a Windows prefix (drive letter / UNC), disallowed here.
    #[error("path contains a disallowed prefix")]
    Prefix,
    /// None of the configured roots matched (multi-root resolution).
    #[error("no configured root matched")]
    NoRoot,
}

/// Resolve `rel` under `root`, returning the joined absolute-ish path only if it
/// stays inside `root`. Lexical: `a/b/../c` → `root/a/c`, but `../x` is rejected.
pub fn resolve_under(root: &Path, rel: &str) -> Result<PathBuf, ScopeError> {
    if rel.as_bytes().contains(&0) {
        return Err(ScopeError::IllegalByte);
    }
    // Backslashes are treated as a traversal/structure smell on the Unix agent;
    // reject to avoid `..\` style escapes being misparsed as a single component.
    if rel.contains('\\') {
        return Err(ScopeError::Traversal);
    }

    let rel_path = Path::new(rel);
    let mut normalized = PathBuf::new();
    for comp in rel_path.components() {
        match comp {
            Component::Prefix(_) => return Err(ScopeError::Prefix),
            Component::RootDir => return Err(ScopeError::Absolute),
            Component::CurDir => {} // `.` is a no-op
            Component::ParentDir => {
                // Refuse to pop above the (virtual) root.
                if !normalized.pop() {
                    return Err(ScopeError::Traversal);
                }
            }
            Component::Normal(seg) => normalized.push(seg),
        }
    }
    Ok(root.join(normalized))
}

/// Resolve `rel` under the first configured root that produces a valid,
/// contained path. Returns [`ScopeError::NoRoot`] if `roots` is empty, and
/// propagates the resolution error otherwise (all roots share the same lexical
/// rules, so a traversal fails identically against each).
pub fn resolve_within_roots(roots: &[PathBuf], rel: &str) -> Result<PathBuf, ScopeError> {
    let first = roots.first().ok_or(ScopeError::NoRoot)?;
    resolve_under(first, rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/Users/yuri/projects/quilomboaraucaria")
    }

    #[test]
    fn plain_relative_ok() {
        let got = resolve_under(&root(), "content/sobre.md").unwrap();
        assert_eq!(got, root().join("content/sobre.md"));
    }

    #[test]
    fn current_dir_segments_collapse() {
        let got = resolve_under(&root(), "./content/./sobre.md").unwrap();
        assert_eq!(got, root().join("content/sobre.md"));
    }

    #[test]
    fn interior_parent_dir_ok() {
        // a/b/../c stays inside the root.
        let got = resolve_under(&root(), "content/draft/../sobre.md").unwrap();
        assert_eq!(got, root().join("content/sobre.md"));
    }

    #[test]
    fn leading_traversal_rejected() {
        assert_eq!(
            resolve_under(&root(), "../secrets").unwrap_err(),
            ScopeError::Traversal
        );
    }

    #[test]
    fn deep_traversal_rejected() {
        assert_eq!(
            resolve_under(&root(), "content/../../../../etc/passwd").unwrap_err(),
            ScopeError::Traversal
        );
    }

    #[test]
    fn absolute_rejected() {
        assert_eq!(
            resolve_under(&root(), "/etc/passwd").unwrap_err(),
            ScopeError::Absolute
        );
    }

    #[test]
    fn backslash_rejected() {
        assert_eq!(
            resolve_under(&root(), "..\\windows").unwrap_err(),
            ScopeError::Traversal
        );
    }

    #[test]
    fn nul_byte_rejected() {
        assert_eq!(
            resolve_under(&root(), "a\0b").unwrap_err(),
            ScopeError::IllegalByte
        );
    }

    #[test]
    fn empty_rel_is_the_root() {
        assert_eq!(resolve_under(&root(), "").unwrap(), root());
    }

    #[test]
    fn multi_root_uses_first() {
        let roots = vec![root(), PathBuf::from("/Users/yuri/projects/ArteLonga")];
        let got = resolve_within_roots(&roots, "content/x.md").unwrap();
        assert_eq!(got, root().join("content/x.md"));
    }

    #[test]
    fn no_roots_errs() {
        assert_eq!(
            resolve_within_roots(&[], "x").unwrap_err(),
            ScopeError::NoRoot
        );
    }
}
