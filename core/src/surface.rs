//! CO-338: Surface keys — `key::path` cross-universe links that resolve to a
//! node's **live deployment URL** at any nesting depth.
//!
//! A *surface key* is a universe's `key`/handle. A *surface ref* `key::subpath`
//! resolves to the physical URL where that node is served, by walking the
//! lineage (`parent` links) up to the nearest deployable ancestor (a universe
//! that carries a `surface_dns`) and prefixing the chain of handles between
//! that ancestor and the node.
//!
//! This is the deployment-resolving complement of CO-153's logical `co://`
//! scheme: `co://` returns `{universe, path}`; `::` returns the physical URL.
//! It reuses CO-277's tree-walk resolution semantics (uniqueness wins;
//! same-depth ambiguity must be qualified through an ancestor).
//!
//! # Resolution algorithm (`resolve_surface_ref`)
//!
//! 1. **Find the node** whose key == `key`.
//!    - exactly one → use it.
//!    - several at different depths → the **outermost (shallowest) wins**.
//!    - several at the *same* (shallowest) depth → ambiguous → error.
//! 2. **Walk up `parent`** to the nearest deployable ancestor (`surface_dns`
//!    set). The node itself counts if it is deployable.
//! 3. **Base** = `https://<ancestor surface_dns>`; **path prefix** = the chain
//!    of handles from that ancestor (exclusive) down to the node.
//! 4. If **no** ancestor is deployable, fall back to the platform default host
//!    (`co.artelonga.com.br`) and prefix the **full** lineage from the root —
//!    this is exactly the pre-promotion state (`comunicacao::` →
//!    `https://co.artelonga.com.br/co/comunicacao`).
//! 5. **Result** = `<base>/<prefix>/<subpath>` (empty `subpath` or `index` →
//!    the base index).

/// Default deployment host for any node with no deployable ancestor — the CO
/// platform serves such universes at `/<full-lineage-path>`.
pub const DEFAULT_SURFACE_HOST: &str = "co.artelonga.com.br";

/// One node in the recursive universe tree, as seen by the resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceNode {
    /// Universe key / handle — the surface key.
    pub key: String,
    /// Parent universe key (`None` = top-level / root).
    pub parent: Option<String>,
    /// Deployment DNS host — set only on deployable units (e.g.
    /// `yggdrasil.artelonga.com.br`). `None` = inherits a deploying ancestor.
    pub surface_dns: Option<String>,
}

impl SurfaceNode {
    pub fn new(
        key: impl Into<String>,
        parent: Option<impl Into<String>>,
        surface_dns: Option<impl Into<String>>,
    ) -> Self {
        Self {
            key: key.into(),
            parent: parent.map(Into::into),
            surface_dns: surface_dns.map(Into::into),
        }
    }
}

/// A registry of surface nodes plus the platform default host.
#[derive(Debug, Clone)]
pub struct SurfaceRegistry {
    nodes: Vec<SurfaceNode>,
    default_host: String,
}

/// The resolution of a `key::subpath` reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSurface {
    /// Live deployment URL.
    pub url: String,
    /// The resolved node's key.
    pub universe: String,
    /// Key of the deployable ancestor whose `surface_dns` provided the base,
    /// or `None` when the platform default host was used.
    pub deployable_ancestor: Option<String>,
}

/// Why a surface ref could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceError {
    /// No node carries this key.
    UnknownKey(String),
    /// Several nodes share this key at the same (shallowest) depth — the ref
    /// must be qualified through an ancestor (`ancestor::key`).
    Ambiguous {
        key: String,
        candidates: Vec<String>,
    },
    /// The input was not a `key::subpath` reference.
    NotASurfaceRef(String),
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SurfaceError::UnknownKey(k) => write!(f, "unknown surface key '{k}'"),
            SurfaceError::Ambiguous { key, candidates } => {
                // Mirrors CO-277's failure mode: qualify through an ancestor.
                write!(
                    f,
                    "ambiguous surface key '{key}' (qualify {key}: ancestor::{key}); \
                     candidates: {}",
                    candidates.join(", ")
                )
            }
            SurfaceError::NotASurfaceRef(s) => {
                write!(f, "not a surface ref '{s}' (expected key::path)")
            }
        }
    }
}

impl std::error::Error for SurfaceError {}

/// Split a `key::subpath` reference into `(key, subpath)`.
///
/// Returns `None` when the string has no `::` separator. The subpath is taken
/// verbatim after the first `::` (so `a::b::c` → key `a`, subpath `b::c`),
/// trimmed of surrounding whitespace and leading/trailing slashes.
pub fn parse_surface_ref(s: &str) -> Option<(String, String)> {
    let (key, subpath) = s.split_once("::")?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let subpath = subpath.trim().trim_matches('/');
    Some((key.to_string(), subpath.to_string()))
}

impl SurfaceRegistry {
    /// Build a registry from nodes, using the CO platform default host.
    pub fn new(nodes: Vec<SurfaceNode>) -> Self {
        Self {
            nodes,
            default_host: DEFAULT_SURFACE_HOST.to_string(),
        }
    }

    /// Build a registry with an explicit default host (e.g. a UAT/staging host).
    pub fn with_default_host(nodes: Vec<SurfaceNode>, default_host: impl Into<String>) -> Self {
        Self {
            nodes,
            default_host: default_host.into(),
        }
    }

    /// First node carrying `key` (lineage lookups; keys are unique in practice).
    fn node_by_key(&self, key: &str) -> Option<&SurfaceNode> {
        self.nodes.iter().find(|n| n.key == key)
    }

    /// Depth of a node = number of ancestors (root = 0). Cycle-guarded.
    fn depth(&self, node: &SurfaceNode) -> usize {
        let mut depth = 0;
        let mut seen = vec![node.key.clone()];
        let mut cur = node.parent.clone();
        while let Some(pk) = cur {
            if seen.contains(&pk) {
                break; // cycle guard
            }
            match self.node_by_key(&pk) {
                Some(p) => {
                    depth += 1;
                    seen.push(pk);
                    cur = p.parent.clone();
                }
                None => break, // dangling parent → treat as root boundary
            }
        }
        depth
    }

    /// Lineage from `node` up to the root, inclusive: `[node, parent, …, root]`.
    fn lineage(&self, node: &SurfaceNode) -> Vec<SurfaceNode> {
        let mut chain = vec![node.clone()];
        let mut seen = vec![node.key.clone()];
        let mut cur = node.parent.clone();
        while let Some(pk) = cur {
            if seen.contains(&pk) {
                break;
            }
            match self.node_by_key(&pk) {
                Some(p) => {
                    chain.push(p.clone());
                    seen.push(pk);
                    cur = p.parent.clone();
                }
                None => break,
            }
        }
        chain
    }

    /// Resolve a parsed `(key, subpath)` to a live deployment URL.
    pub fn resolve_surface_ref(
        &self,
        key: &str,
        subpath: &str,
    ) -> Result<ResolvedSurface, SurfaceError> {
        // 1. Find candidate nodes by key.
        let candidates: Vec<&SurfaceNode> = self.nodes.iter().filter(|n| n.key == key).collect();
        if candidates.is_empty() {
            return Err(SurfaceError::UnknownKey(key.to_string()));
        }

        // Outermost (shallowest depth) wins; same-depth collision = ambiguous.
        let min_depth = candidates.iter().map(|n| self.depth(n)).min().unwrap();
        let winners: Vec<&SurfaceNode> = candidates
            .iter()
            .copied()
            .filter(|n| self.depth(n) == min_depth)
            .collect();
        if winners.len() > 1 {
            return Err(SurfaceError::Ambiguous {
                key: key.to_string(),
                candidates: winners.iter().map(|n| n.key.clone()).collect(),
            });
        }
        let node = winners[0];

        // 2. Walk lineage to the nearest deployable ancestor (node counts).
        let lineage = self.lineage(node); // [node, …, root]
        let deployable_idx = lineage.iter().position(|n| n.surface_dns.is_some());

        let (base, prefix, deployable_ancestor) = match deployable_idx {
            Some(idx) => {
                let ancestor = &lineage[idx];
                let dns = ancestor.surface_dns.as_deref().unwrap();
                // Handles from the ancestor (exclusive) down to the node.
                // lineage[0..idx] is [node, …, ancestor's child]; reverse for top-down.
                let mut prefix_nodes: Vec<&SurfaceNode> = lineage[..idx].iter().collect();
                prefix_nodes.reverse();
                let prefix = prefix_nodes
                    .iter()
                    .map(|n| n.key.as_str())
                    .collect::<Vec<_>>()
                    .join("/");
                (format!("https://{dns}"), prefix, Some(ancestor.key.clone()))
            }
            None => {
                // No deployable ancestor → platform default host, full lineage.
                let mut full: Vec<&SurfaceNode> = lineage.iter().collect();
                full.reverse(); // [root, …, node]
                let prefix = full
                    .iter()
                    .map(|n| n.key.as_str())
                    .collect::<Vec<_>>()
                    .join("/");
                (format!("https://{}", self.default_host), prefix, None)
            }
        };

        Ok(ResolvedSurface {
            url: build_url(&base, &prefix, subpath),
            universe: node.key.clone(),
            deployable_ancestor,
        })
    }

    /// Convenience: parse + resolve a full `key::subpath` ref string.
    pub fn resolve(&self, reference: &str) -> Result<ResolvedSurface, SurfaceError> {
        let (key, subpath) = parse_surface_ref(reference)
            .ok_or_else(|| SurfaceError::NotASurfaceRef(reference.to_string()))?;
        self.resolve_surface_ref(&key, &subpath)
    }
}

/// Rewrite `[[key::subpath]]` (and `[[key::subpath|label]]`) wikilinks in
/// markdown body text to ordinary markdown links `[label](resolved-url)`.
///
/// This is the render-time half of CO-338: a downstream markdown→HTML pass then
/// turns the emitted `[label](url)` into an `<a href="url">label</a>`. The
/// canonical cross-universe body syntax is the wikilink (`[[key::path]]`), so
/// only `[[…]]` spans whose inner target parses as a surface ref *and* resolves
/// are rewritten — every other wikilink (and any unresolvable surface ref) is
/// left exactly as it was, never corrupting unrelated content or code.
pub fn rewrite_surface_links(text: &str, registry: &SurfaceRegistry) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find("[[") {
        // Emit everything up to the wikilink opener verbatim.
        out.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];

        let Some(end) = after_open.find("]]") else {
            // No closing `]]` — emit the opener and stop scanning for links.
            out.push_str(&rest[start..]);
            return out;
        };

        let inner = &after_open[..end];
        let (target, label) = match inner.split_once('|') {
            Some((t, l)) => (t.trim(), l.trim().to_string()),
            None => (inner.trim(), inner.trim().to_string()),
        };

        // Only rewrite when the inner target is a surface ref that resolves.
        let rewritten = parse_surface_ref(target)
            .and_then(|(key, sub)| registry.resolve_surface_ref(&key, &sub).ok())
            .map(|r| format!("[{label}]({})", r.url));

        match rewritten {
            Some(link) => out.push_str(&link),
            // Leave the original `[[…]]` span untouched.
            None => {
                out.push_str("[[");
                out.push_str(inner);
                out.push_str("]]");
            }
        }

        rest = &after_open[end + 2..];
    }

    out.push_str(rest);
    out
}

/// Join `base` + `prefix` + `subpath` into a URL. Empty/`index` subpath and an
/// empty prefix collapse to the base index (`https://host/`).
fn build_url(base: &str, prefix: &str, subpath: &str) -> String {
    let sub = subpath.trim().trim_matches('/');
    let sub = if sub.is_empty() || sub == "index" {
        ""
    } else {
        sub
    };

    let path = match (prefix.is_empty(), sub.is_empty()) {
        (true, true) => String::new(),
        (true, false) => sub.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}/{sub}"),
    };
    format!("{base}/{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry matching the I-Need narrative: yggdrasil → comunicacao → mbya,
    /// plus the standalone promoted `yuri` surface.
    fn yggdrasil_registry() -> SurfaceRegistry {
        SurfaceRegistry::new(vec![
            SurfaceNode::new(
                "yggdrasil",
                None::<String>,
                Some("yggdrasil.artelonga.com.br"),
            ),
            SurfaceNode::new("comunicacao", Some("yggdrasil"), None::<String>),
            SurfaceNode::new("mbya", Some("comunicacao"), None::<String>),
            SurfaceNode::new("yuri", None::<String>, Some("yuri.artelonga.com.br")),
        ])
    }

    fn resolve(reg: &SurfaceRegistry, r: &str) -> String {
        reg.resolve(r).expect("resolves").url
    }

    // --- The six worked-example rows from the spec ---

    #[test]
    fn row1_yggdrasil_index() {
        let reg = yggdrasil_registry();
        assert_eq!(
            resolve(&reg, "yggdrasil::index"),
            "https://yggdrasil.artelonga.com.br/"
        );
    }

    #[test]
    fn row2_yggdrasil_explicit_subpath() {
        let reg = yggdrasil_registry();
        assert_eq!(
            resolve(&reg, "yggdrasil::comunicacao/mbya"),
            "https://yggdrasil.artelonga.com.br/comunicacao/mbya"
        );
    }

    #[test]
    fn row3_mbya_bare_walks_to_yggdrasil() {
        let reg = yggdrasil_registry();
        // `mbya::` resolves identically to its fully-qualified form.
        assert_eq!(
            resolve(&reg, "mbya::"),
            "https://yggdrasil.artelonga.com.br/comunicacao/mbya"
        );
        assert_eq!(
            resolve(&reg, "mbya::"),
            resolve(&reg, "yggdrasil::comunicacao/mbya")
        );
    }

    #[test]
    fn row4_yuri_own_surface() {
        let reg = yggdrasil_registry();
        assert_eq!(resolve(&reg, "yuri::"), "https://yuri.artelonga.com.br/");
    }

    #[test]
    fn row5_comunicacao_pre_promotion() {
        // co (no dns) → comunicacao falls back to the platform default host,
        // prefixing the full lineage `co/comunicacao`.
        let reg = SurfaceRegistry::new(vec![
            SurfaceNode::new("co", None::<String>, None::<String>),
            SurfaceNode::new("comunicacao", Some("co"), None::<String>),
        ]);
        let r = reg.resolve("comunicacao::").expect("resolves");
        assert_eq!(r.url, "https://co.artelonga.com.br/co/comunicacao");
        assert_eq!(r.deployable_ancestor, None);
    }

    #[test]
    fn row6_comunicacao_post_promotion() {
        let reg = SurfaceRegistry::new(vec![
            SurfaceNode::new("co", None::<String>, None::<String>),
            SurfaceNode::new(
                "comunicacao",
                Some("co"),
                Some("comunicacao.artelonga.com.br"),
            ),
        ]);
        let r = reg.resolve("comunicacao::").expect("resolves");
        assert_eq!(r.url, "https://comunicacao.artelonga.com.br/");
        assert_eq!(r.deployable_ancestor.as_deref(), Some("comunicacao"));
    }

    // --- Promotion test (acceptance criterion 6) ---

    #[test]
    fn promotion_changes_every_resolution_with_no_ref_edits() {
        let pre = SurfaceRegistry::new(vec![
            SurfaceNode::new("co", None::<String>, None::<String>),
            SurfaceNode::new("comunicacao", Some("co"), None::<String>),
        ]);
        // Same refs, evaluated against the pre-promotion data.
        assert_eq!(
            pre.resolve("comunicacao::").unwrap().url,
            "https://co.artelonga.com.br/co/comunicacao"
        );
        assert_eq!(
            pre.resolve("comunicacao::guia/intro").unwrap().url,
            "https://co.artelonga.com.br/co/comunicacao/guia/intro"
        );

        // Flip ONLY the data: comunicacao gains its own surface_dns.
        let post = SurfaceRegistry::new(vec![
            SurfaceNode::new("co", None::<String>, None::<String>),
            SurfaceNode::new(
                "comunicacao",
                Some("co"),
                Some("comunicacao.artelonga.com.br"),
            ),
        ]);
        // The identical refs now follow the surface — zero ref edits.
        assert_eq!(
            post.resolve("comunicacao::").unwrap().url,
            "https://comunicacao.artelonga.com.br/"
        );
        assert_eq!(
            post.resolve("comunicacao::guia/intro").unwrap().url,
            "https://comunicacao.artelonga.com.br/guia/intro"
        );
    }

    // --- Shadowing + ambiguity (acceptance criterion 1) ---

    #[test]
    fn shadowing_outermost_wins() {
        // `mbya` appears at depth 1 (under root `a`) and depth 2 (under `b`,
        // under root `c`). The bare key resolves to the shallower (outermost).
        let reg = SurfaceRegistry::new(vec![
            SurfaceNode::new("a", None::<String>, Some("a.example.com")),
            SurfaceNode::new("c", None::<String>, Some("c.example.com")),
            SurfaceNode::new("b", Some("c"), None::<String>),
            SurfaceNode::new("mbya", Some("a"), None::<String>), // depth 1
            SurfaceNode::new("mbya", Some("b"), None::<String>), // depth 2 (shadowed)
        ]);
        // Outermost = the depth-1 mbya under deployable `a`.
        assert_eq!(
            reg.resolve("mbya::").unwrap().url,
            "https://a.example.com/mbya"
        );
    }

    #[test]
    fn same_depth_collision_is_ambiguous() {
        let reg = SurfaceRegistry::new(vec![
            SurfaceNode::new("a", None::<String>, Some("a.example.com")),
            SurfaceNode::new("b", None::<String>, Some("b.example.com")),
            SurfaceNode::new("dup", Some("a"), None::<String>), // depth 1
            SurfaceNode::new("dup", Some("b"), None::<String>), // depth 1 → collision
        ]);
        let err = reg.resolve("dup::").unwrap_err();
        match err {
            SurfaceError::Ambiguous { key, .. } => assert_eq!(key, "dup"),
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn unknown_key_errors() {
        let reg = yggdrasil_registry();
        assert_eq!(
            reg.resolve("nope::").unwrap_err(),
            SurfaceError::UnknownKey("nope".to_string())
        );
    }

    #[test]
    fn parse_surface_ref_splits_on_first_separator() {
        assert_eq!(
            parse_surface_ref("mbya::a/b"),
            Some(("mbya".into(), "a/b".into()))
        );
        assert_eq!(
            parse_surface_ref("mbya::"),
            Some(("mbya".into(), "".into()))
        );
        assert_eq!(
            parse_surface_ref("a::b::c"),
            Some(("a".into(), "b::c".into()))
        );
        assert_eq!(parse_surface_ref("no-separator"), None);
        assert_eq!(parse_surface_ref("::x"), None);
    }

    // --- Markdown wikilink rewriting (acceptance criterion 5) ---

    #[test]
    fn rewrite_surface_links_resolves_wikilinks() {
        let reg = yggdrasil_registry();
        let md = "See [[mbya::]] and [[yggdrasil::comunicacao/mbya|the page]].";
        let out = rewrite_surface_links(md, &reg);
        assert_eq!(
            out,
            "See [mbya::](https://yggdrasil.artelonga.com.br/comunicacao/mbya) \
             and [the page](https://yggdrasil.artelonga.com.br/comunicacao/mbya)."
        );
    }

    #[test]
    fn rewrite_surface_links_leaves_non_surface_and_unknown_untouched() {
        let reg = yggdrasil_registry();
        // A plain wikilink (no `::`) and an unknown key are both left as-is.
        let md = "[[notas/intro]] and [[nope::x]] stay.";
        assert_eq!(rewrite_surface_links(md, &reg), md);
        // An unterminated `[[` is emitted verbatim.
        assert_eq!(
            rewrite_surface_links("dangling [[mbya::", &reg),
            "dangling [[mbya::"
        );
    }

    #[test]
    fn cycle_in_parent_chain_is_guarded() {
        // x → y → x (cycle). Resolution must terminate, not hang.
        let reg = SurfaceRegistry::new(vec![
            SurfaceNode::new("x", Some("y"), None::<String>),
            SurfaceNode::new("y", Some("x"), None::<String>),
        ]);
        let r = reg.resolve("x::").unwrap();
        // No deployable ancestor → default host, full (cycle-truncated) lineage.
        assert!(r.url.starts_with("https://co.artelonga.com.br/"));
    }
}
