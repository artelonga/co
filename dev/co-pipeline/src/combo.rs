//! CO-88 — the layer combinations exercised by the pipeline matrix.
//!
//! Each combo is a point in the (compression × encryption × signing) space
//! plus the `legacy-md` backwards-compat path where the receiver auto-wraps a
//! bare markdown body into a [`CoFile`](co::proto::sync::CoFile) envelope.

use serde::Serialize;

/// One layer combination from the CO-88 matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Combo {
    /// No layers — protobuf `CoFile` envelope only.
    Raw,
    /// zstd-3 compression.
    Zstd,
    /// Ed25519 signature, no compression/encryption.
    Signed,
    /// zstd-3 + ChaCha20-Poly1305 + Ed25519 — the "real-world" combo.
    Private,
    /// Bare markdown bytes; the receiver auto-wraps into a `CoFile`.
    LegacyMd,
}

impl Combo {
    /// All five combos, in report order.
    pub const ALL: [Combo; 5] = [
        Combo::Raw,
        Combo::Zstd,
        Combo::Signed,
        Combo::Private,
        Combo::LegacyMd,
    ];

    /// Stable kebab-case label used in reports and telemetry.
    pub fn label(self) -> &'static str {
        match self {
            Combo::Raw => "raw",
            Combo::Zstd => "zstd",
            Combo::Signed => "signed",
            Combo::Private => "private",
            Combo::LegacyMd => "legacy-md",
        }
    }

    /// Parse a label back into a [`Combo`].
    pub fn from_label(s: &str) -> Option<Combo> {
        Combo::ALL.into_iter().find(|c| c.label() == s)
    }

    /// Whether this combo zstd-compresses the envelope.
    pub fn compresses(self) -> bool {
        matches!(self, Combo::Zstd | Combo::Private)
    }

    /// Whether this combo encrypts the envelope with ChaCha20-Poly1305.
    pub fn encrypts(self) -> bool {
        matches!(self, Combo::Private)
    }

    /// Whether this combo appends an Ed25519 signature.
    pub fn signs(self) -> bool {
        matches!(self, Combo::Signed | Combo::Private)
    }

    /// Whether this combo skips the `CoFile` envelope and ships bare markdown.
    pub fn is_legacy(self) -> bool {
        matches!(self, Combo::LegacyMd)
    }

    /// Telemetry `compression` label (matches CO-88b column values).
    pub fn compression_label(self) -> &'static str {
        if self.compresses() { "zstd-3" } else { "raw" }
    }

    /// Telemetry `encryption` label.
    pub fn encryption_label(self) -> &'static str {
        if self.encrypts() { "chacha20" } else { "none" }
    }

    /// Telemetry `co_format` label.
    pub fn format_label(self) -> &'static str {
        if self.is_legacy() {
            "markdown"
        } else {
            "co/protobuf"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_roundtrip() {
        for c in Combo::ALL {
            assert_eq!(Combo::from_label(c.label()), Some(c));
        }
        assert_eq!(Combo::from_label("nope"), None);
    }

    #[test]
    fn layer_flags() {
        assert!(!Combo::Raw.compresses() && !Combo::Raw.signs());
        assert!(Combo::Zstd.compresses() && !Combo::Zstd.signs());
        assert!(Combo::Signed.signs() && !Combo::Signed.compresses());
        assert!(Combo::Private.compresses() && Combo::Private.encrypts() && Combo::Private.signs());
        assert!(Combo::LegacyMd.is_legacy());
    }
}
