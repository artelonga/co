//! CO-86 — the `.co` file format: protobuf-wrapped markdown for
//! transport-optimized, encrypted, self-describing content.
//!
//! Markdown is the *authoring* surface; `.co` is the *wire* surface. A `.co`
//! file is a length-prefixed [`CoFile`] protobuf preceded by the 4-byte magic
//! [`MAGIC`] (`co\x01\x00`) so a reader can discriminate it from a bare `.md`
//! in O(1). A plain UTF-8 markdown file (no magic) is *auto-wrapped* on read
//! into a default [`CoFile`], so existing universes work without re-import.
//!
//! ```text
//! .md ──from_markdown──> CoFile { frontmatter, body=markdown(..) } ──to_bytes──> bytes (.co)
//! bytes (.co) ──from_bytes──> CoFile ──to_markdown──> .md (byte-identical body)
//! ```
//!
//! Composable layers wrap the *same* envelope (CO-87 makes each a trait):
//! - [`codec`] — zstd compression of the body
//! - [`encryption`] — chacha20-poly1305 body encryption
//! - [`signature`] — Ed25519 detached signatures
//! - [`telemetry`] — wire-size / codec accounting populated on encode

use sha2::{Digest, Sha256};

pub mod codec;
pub mod encryption;
pub mod signature;
pub mod telemetry;

// Re-export the generated protobuf types so callers use `co::co_format::CoFile`
// rather than reaching into `co::proto::co_format`.
pub use crate::proto::co_format::{
    Attachment, AttachmentSource, CoFile, Composite, Encrypted, Encryption, Form, Frontmatter,
    Signature, Telemetry, attachment_source, co_file,
};

/// Schema version emitted by this encoder.
pub const SCHEMA_VERSION: &str = "1.0.0";

/// Magic bytes prefixing every `.co` file (`co\x01\x00`). Lets a reader
/// distinguish a `.co` envelope from bare markdown without a full decode.
pub const MAGIC: [u8; 4] = *b"co\x01\x00";

/// MIME type for `.co` envelopes (IANA registration deferred).
pub const MIME_TYPE: &str = "application/vnd.co+protobuf";

/// Errors raised while encoding/decoding/validating a `.co` envelope.
#[derive(Debug, thiserror::Error)]
pub enum CoError {
    #[error("protobuf decode failed: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("zstd codec error: {0}")]
    Codec(#[source] std::io::Error),
    #[error("body is encrypted; decrypt before accessing the markdown")]
    BodyEncrypted,
    #[error("body is empty; nothing to decode")]
    EmptyBody,
    #[error("content hash mismatch: expected {expected}, got {actual} (tampering or corruption)")]
    HashMismatch { expected: String, actual: String },
    #[error("decoded body is not valid UTF-8 markdown")]
    NotUtf8,
    #[error("encryption error: {0}")]
    Crypto(String),
    #[error("invalid envelope: {0}")]
    Invalid(String),
}

/// SHA-256 of `bytes`, lowercase hex — the canonical [`CoFile::content_hash`].
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ---------------------------------------------------------------------------
// Markdown → CoFile
// ---------------------------------------------------------------------------

/// Build a [`CoFile`] from authored markdown. Frontmatter is parsed into the
/// typed [`Frontmatter`] message (recognized keys) plus `extra` (the rest);
/// the markdown *body* is stored verbatim in `body.markdown` so a later
/// [`to_markdown`] reproduces it byte-for-byte. Timestamps default to 0 and
/// can be set by the caller for determinism.
pub fn from_markdown(md: &str) -> CoFile {
    let (yaml, body) = split_frontmatter(md);
    let frontmatter = yaml.map(parse_frontmatter);
    let body_bytes = body.as_bytes().to_vec();
    let hash = content_hash(&body_bytes);

    CoFile {
        version: SCHEMA_VERSION.to_string(),
        content_type: "co/markdown".to_string(),
        content_hash: hash,
        created_at_ns: 0,
        modified_at_ns: 0,
        author_id: String::new(),
        universe_key: String::new(),
        entry_path: String::new(),
        frontmatter,
        form: None,
        body: Some(co_file::Body::Markdown(body_bytes)),
        attachments: Vec::new(),
        signature: None,
        telemetry: None,
    }
}

/// Reconstruct authored markdown from a [`CoFile`]. The body is byte-identical
/// to what [`from_markdown`] consumed; the frontmatter is re-emitted from the
/// typed message (equivalent, not necessarily byte-identical YAML).
pub fn to_markdown(co: &CoFile) -> Result<String, CoError> {
    let body = canonical_body(co)?;
    let body = String::from_utf8(body).map_err(|_| CoError::NotUtf8)?;

    let yaml = co.frontmatter.as_ref().and_then(emit_frontmatter);
    match yaml {
        Some(y) if !y.is_empty() => Ok(format!("---\n{y}---\n{body}")),
        _ => Ok(body),
    }
}

/// The plaintext markdown body bytes, regardless of how the body is stored
/// (`markdown` verbatim, `compressed` via zstd, or `composite`). Returns
/// [`CoError::BodyEncrypted`] for an encrypted body — decrypt it first.
pub fn canonical_body(co: &CoFile) -> Result<Vec<u8>, CoError> {
    match co.body.as_ref() {
        Some(co_file::Body::Markdown(b)) => Ok(b.clone()),
        Some(co_file::Body::Compressed(b)) => codec::zstd_decompress(b),
        Some(co_file::Body::Composite(c)) => Ok(c.markdown.clone()),
        Some(co_file::Body::Encrypted(_)) => Err(CoError::BodyEncrypted),
        None => Err(CoError::EmptyBody),
    }
}

// ---------------------------------------------------------------------------
// CoFile <-> bytes (.co)
// ---------------------------------------------------------------------------

/// Serialize a [`CoFile`] to `.co` bytes: [`MAGIC`] followed by the protobuf.
pub fn to_bytes(co: &CoFile) -> Vec<u8> {
    use prost::Message;
    let mut out = Vec::with_capacity(4 + co.encoded_len());
    out.extend_from_slice(&MAGIC);
    co.encode(&mut out).expect("Vec<u8> never fails to write");
    out
}

/// Parse `.co` bytes. With the magic prefix it decodes the protobuf; without
/// it the input is treated as bare UTF-8 markdown and auto-wrapped (CO-86's
/// backward-compat decoder), so legacy `.md` flows transparently.
pub fn from_bytes(bytes: &[u8]) -> Result<CoFile, CoError> {
    use prost::Message;
    if bytes.len() >= 4 && bytes[..4] == MAGIC {
        Ok(CoFile::decode(&bytes[4..])?)
    } else {
        let md = std::str::from_utf8(bytes).map_err(|_| CoError::NotUtf8)?;
        Ok(from_markdown(md))
    }
}

/// Returns `true` if `bytes` begin with the `.co` [`MAGIC`].
pub fn is_co_file(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == MAGIC
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate envelope invariants: non-empty `version`, a present body, and —
/// when the body is plaintext — that `content_hash` matches the body bytes.
pub fn validate(co: &CoFile) -> Result<(), CoError> {
    if co.version.is_empty() {
        return Err(CoError::Invalid("missing version".into()));
    }
    if co.body.is_none() {
        return Err(CoError::EmptyBody);
    }
    // Hash check only when the plaintext body is available.
    match canonical_body(co) {
        Ok(body) => {
            let actual = content_hash(&body);
            if actual != co.content_hash {
                return Err(CoError::HashMismatch {
                    expected: co.content_hash.clone(),
                    actual,
                });
            }
            Ok(())
        }
        Err(CoError::BodyEncrypted) => Ok(()),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Inspection (for `co inspect`)
// ---------------------------------------------------------------------------

/// A human-readable summary of a `.co` envelope, surfaced by `co inspect`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    /// Envelope schema version.
    pub version: String,
    /// Declared content type (`co/markdown`, `co/encrypted`, …).
    pub content_type: String,
    /// Plaintext body size in bytes (post-decompress; ciphertext size when encrypted).
    pub size_uncompressed: i64,
    /// Total bytes on the wire (magic + protobuf).
    pub size_on_wire: i64,
    /// Whether a signature is attached.
    pub signed: bool,
    /// Whether the body is encrypted.
    pub encrypted: bool,
    /// Number of attachments in the graph.
    pub attachment_count: usize,
}

/// Summarize a [`CoFile`] without materializing the markdown. `size_on_wire` is
/// supplied by the caller (the `.co` byte length it just read or wrote).
pub fn inspect(co: &CoFile, size_on_wire: usize) -> Inspection {
    let encrypted = matches!(co.body, Some(co_file::Body::Encrypted(_)));
    let size_uncompressed = match co.body.as_ref() {
        Some(co_file::Body::Markdown(b)) => b.len() as i64,
        Some(co_file::Body::Composite(c)) => c.markdown.len() as i64,
        Some(co_file::Body::Compressed(b)) => codec::zstd_decompress(b)
            .map(|d| d.len() as i64)
            .unwrap_or(b.len() as i64),
        Some(co_file::Body::Encrypted(e)) => e.ciphertext.len() as i64,
        None => 0,
    };
    Inspection {
        version: co.version.clone(),
        content_type: co.content_type.clone(),
        size_uncompressed,
        size_on_wire: size_on_wire as i64,
        signed: co.signature.is_some(),
        encrypted,
        attachment_count: co.attachments.len(),
    }
}

// ---------------------------------------------------------------------------
// Frontmatter <-> typed message
// ---------------------------------------------------------------------------

/// Split `md` into `(Some(yaml), body)` when it opens with a `---` frontmatter
/// block, else `(None, md)`. `yaml` excludes the delimiters; `body` is the
/// remainder verbatim (so it round-trips byte-for-byte).
fn split_frontmatter(md: &str) -> (Option<&str>, &str) {
    let Some(rest) = md.strip_prefix("---\n") else {
        return (None, md);
    };
    if let Some(idx) = rest.find("\n---\n") {
        return (Some(&rest[..idx]), &rest[idx + 5..]);
    }
    // Closing fence at EOF with no trailing body.
    if let Some(stripped) = rest.strip_suffix("\n---") {
        return (Some(stripped), "");
    }
    (None, md)
}

/// Parse YAML frontmatter into the typed [`Frontmatter`] message. Recognized
/// keys (`title`, `description`, `type`, `tags`, `status`, `priority`,
/// `parent`, `assignee`) map to typed fields; everything else lands in `extra`
/// and is re-emitted under the same key, so CO content round-trips.
fn parse_frontmatter(yaml: &str) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let value: serde_yaml::Value = match serde_yaml::from_str(yaml) {
        Ok(v) => v,
        Err(_) => return fm, // unparseable frontmatter → empty typed view
    };
    let Some(map) = value.as_mapping() else {
        return fm;
    };

    let mut extra = prost_types::Struct::default();
    for (k, v) in map {
        let Some(key) = k.as_str() else { continue };

        // A recognized key is promoted to its typed field ONLY when its value
        // is a non-empty string (or, for `tags`, a non-empty string list).
        // Empty/null/odd-typed values fall through to `extra` so they round-trip
        // faithfully (e.g. `description: ""`, `tags: []`, `parent: null`).
        let promoted = match (key, v) {
            ("title", serde_yaml::Value::String(s)) if !s.is_empty() => {
                fm.title = s.clone();
                true
            }
            ("description", serde_yaml::Value::String(s)) if !s.is_empty() => {
                fm.description = s.clone();
                true
            }
            ("type", serde_yaml::Value::String(s)) if !s.is_empty() => {
                fm.entry_type = s.clone();
                true
            }
            ("status", serde_yaml::Value::String(s)) if !s.is_empty() => {
                fm.status = s.clone();
                true
            }
            ("priority", serde_yaml::Value::String(s)) if !s.is_empty() => {
                fm.priority = s.clone();
                true
            }
            ("parent", serde_yaml::Value::String(s)) if !s.is_empty() => {
                fm.parent = s.clone();
                true
            }
            ("assignee", serde_yaml::Value::String(s)) if !s.is_empty() => {
                fm.assignee = s.clone();
                true
            }
            ("tags", serde_yaml::Value::Sequence(seq)) if !seq.is_empty() => {
                fm.tags = seq.iter().map(yaml_to_string).collect();
                true
            }
            _ => false,
        };

        if !promoted {
            extra.fields.insert(key.to_string(), yaml_to_prost_value(v));
        }
    }
    if !extra.fields.is_empty() {
        fm.extra = Some(extra);
    }
    fm
}

/// Re-emit YAML frontmatter from the typed message. Returns `None` when there
/// is nothing to emit. The trailing newline is included so callers can wrap it
/// directly between `---` fences.
fn emit_frontmatter(fm: &Frontmatter) -> Option<String> {
    let mut map = serde_yaml::Mapping::new();
    let put = |m: &mut serde_yaml::Mapping, k: &str, v: serde_yaml::Value| {
        m.insert(serde_yaml::Value::String(k.to_string()), v);
    };

    if !fm.title.is_empty() {
        put(&mut map, "title", fm.title.clone().into());
    }
    if !fm.description.is_empty() {
        put(&mut map, "description", fm.description.clone().into());
    }
    if !fm.entry_type.is_empty() {
        put(&mut map, "type", fm.entry_type.clone().into());
    }
    if !fm.tags.is_empty() {
        let seq = fm
            .tags
            .iter()
            .map(|t| serde_yaml::Value::String(t.clone()))
            .collect();
        put(&mut map, "tags", serde_yaml::Value::Sequence(seq));
    }
    if !fm.status.is_empty() {
        put(&mut map, "status", fm.status.clone().into());
    }
    if !fm.priority.is_empty() {
        put(&mut map, "priority", fm.priority.clone().into());
    }
    if !fm.parent.is_empty() {
        put(&mut map, "parent", fm.parent.clone().into());
    }
    if !fm.assignee.is_empty() {
        put(&mut map, "assignee", fm.assignee.clone().into());
    }
    if let Some(extra) = fm.extra.as_ref() {
        for (k, v) in &extra.fields {
            put(&mut map, k, prost_value_to_yaml(v));
        }
    }

    if map.is_empty() {
        return None;
    }
    serde_yaml::to_string(&serde_yaml::Value::Mapping(map)).ok()
}

/// Parse the frontmatter of a markdown string into a `serde_yaml::Value`
/// mapping. Used by round-trip tests to assert *frontmatter equivalence*.
pub fn frontmatter_value(md: &str) -> serde_yaml::Value {
    match split_frontmatter(md).0 {
        Some(yaml) => serde_yaml::from_str(yaml).unwrap_or(serde_yaml::Value::Null),
        None => serde_yaml::Value::Null,
    }
}

/// The markdown body of `md` — everything after a leading frontmatter block,
/// or all of `md` when there is none. Used by round-trip tests to assert the
/// body is preserved byte-for-byte.
pub fn body_str(md: &str) -> &str {
    split_frontmatter(md).1
}

fn yaml_to_string(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        _ => serde_yaml::to_string(v)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

// ---- serde_yaml::Value <-> prost_types::Value -----------------------------

fn yaml_to_prost_value(v: &serde_yaml::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match v {
        serde_yaml::Value::Null => Kind::NullValue(0),
        serde_yaml::Value::Bool(b) => Kind::BoolValue(*b),
        serde_yaml::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_yaml::Value::String(s) => Kind::StringValue(s.clone()),
        serde_yaml::Value::Sequence(seq) => Kind::ListValue(prost_types::ListValue {
            values: seq.iter().map(yaml_to_prost_value).collect(),
        }),
        serde_yaml::Value::Mapping(map) => {
            let mut s = prost_types::Struct::default();
            for (k, val) in map {
                if let Some(key) = k.as_str() {
                    s.fields.insert(key.to_string(), yaml_to_prost_value(val));
                }
            }
            Kind::StructValue(s)
        }
        serde_yaml::Value::Tagged(t) => return yaml_to_prost_value(&t.value),
    };
    prost_types::Value { kind: Some(kind) }
}

fn prost_value_to_yaml(v: &prost_types::Value) -> serde_yaml::Value {
    use prost_types::value::Kind;
    match &v.kind {
        None | Some(Kind::NullValue(_)) => serde_yaml::Value::Null,
        Some(Kind::BoolValue(b)) => serde_yaml::Value::Bool(*b),
        Some(Kind::NumberValue(n)) => {
            // Recover integers: protobuf Struct numbers are always f64, but YAML
            // distinguishes int from float — emit an int when the value is whole.
            if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
                serde_yaml::Value::Number((*n as i64).into())
            } else {
                serde_yaml::Value::Number((*n).into())
            }
        }
        Some(Kind::StringValue(s)) => serde_yaml::Value::String(s.clone()),
        Some(Kind::ListValue(l)) => {
            serde_yaml::Value::Sequence(l.values.iter().map(prost_value_to_yaml).collect())
        }
        Some(Kind::StructValue(s)) => {
            let mut map = serde_yaml::Mapping::new();
            for (k, val) in &s.fields {
                map.insert(
                    serde_yaml::Value::String(k.clone()),
                    prost_value_to_yaml(val),
                );
            }
            serde_yaml::Value::Mapping(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_body_byte_identical_and_frontmatter_equivalent() {
        let md = "---\ntitle: Sobre\ntype: page\ntags:\n- a\n- b\norder: 3\n---\n# Olá\n\nCorpo do texto.\n";
        let co = from_markdown(md);
        let out = to_markdown(&co).unwrap();

        // Body byte-identical.
        let orig_body = split_frontmatter(md).1;
        let out_body = split_frontmatter(&out).1;
        assert_eq!(orig_body, out_body);

        // Frontmatter equivalent (order-independent).
        assert_eq!(frontmatter_value(md), frontmatter_value(&out));
    }

    #[test]
    fn bytes_roundtrip_with_magic() {
        let md = "---\ntitle: T\n---\nbody\n";
        let co = from_markdown(md);
        let bytes = to_bytes(&co);
        assert!(is_co_file(&bytes));
        let back = from_bytes(&bytes).unwrap();
        assert_eq!(to_markdown(&back).unwrap(), to_markdown(&co).unwrap());
    }

    #[test]
    fn auto_wrap_bare_markdown() {
        let md = "# No frontmatter\n\njust text";
        let co = from_bytes(md.as_bytes()).unwrap();
        assert_eq!(to_markdown(&co).unwrap(), md);
        assert_eq!(co.content_type, "co/markdown");
    }

    #[test]
    fn no_frontmatter_roundtrip() {
        let md = "plain body, no fences\n";
        let co = from_markdown(md);
        assert_eq!(to_markdown(&co).unwrap(), md);
        assert!(co.frontmatter.is_none());
    }

    #[test]
    fn typed_fields_populated() {
        let md = "---\ntitle: Task X\ntype: task\nstatus: todo\npriority: high\ntags:\n- urgent\n---\nDo it.\n";
        let co = from_markdown(md);
        let fm = co.frontmatter.as_ref().unwrap();
        assert_eq!(fm.title, "Task X");
        assert_eq!(fm.entry_type, "task");
        assert_eq!(fm.status, "todo");
        assert_eq!(fm.priority, "high");
        assert_eq!(fm.tags, vec!["urgent".to_string()]);
    }

    #[test]
    fn validate_detects_tamper() {
        let md = "---\ntitle: T\n---\nbody\n";
        let mut co = from_markdown(md);
        validate(&co).unwrap();
        co.content_hash = "deadbeef".into();
        assert!(matches!(validate(&co), Err(CoError::HashMismatch { .. })));
    }

    #[test]
    fn extra_int_roundtrips_as_int() {
        let md = "---\ncount: 42\nratio: 1.5\n---\nx\n";
        let co = from_markdown(md);
        let out = to_markdown(&co).unwrap();
        assert_eq!(frontmatter_value(md), frontmatter_value(&out));
    }
}
