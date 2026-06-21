//! CO-87 — Composable protocol stack.
//!
//! Every read/write of a [`CoFile`](crate::proto::sync::CoFile) crosses the same
//! conceptual layers — physical (disk/net) → cache → storage → network →
//! privacy (encryption) → security (signing). This module exposes each concern
//! as a [`Layer`] whose input and output is a `CoFile` (or a transform thereof)
//! and a [`Stack`] composer that chains two layers so the bottom's output feeds
//! the top's input.
//!
//! ## The model (v1 — "all `CoFile`")
//!
//! A layer transforms a `CoFile` going one direction and reverses it on the
//! other:
//!
//! * **write** flows *top → bottom* (consumer → durable home). Each transform
//!   mutates the `content` on the way down; the bottom layer persists/ships the
//!   resulting bytes.
//! * **read** flows *bottom → top* (durable home → consumer). The bottom layer
//!   produces bytes; each transform reverses its write on the way up.
//!
//! A *read request* `CoFile` carries the addressing metadata (its `path`, plus
//! any sidecar fields the caller already knows); terminal layers fill in
//! `content`. This keeps every layer a uniform `CoFile → CoFile` map so [`Stack`]
//! composition is trivial, and makes round-trips bit-perfect on `content`.
//!
//! ## Composition rules (enforced by build order, see the task spec)
//!
//! * **Privacy below Security** — sign the ciphertext, not the plaintext.
//! * **Compression below Encryption** — compress first, encrypt the compressed
//!   bytes.
//! * **Cache above Storage / Transport** — caches sit closer to the consumer.
//!
//! Example 5-layer writer (bytes on disk are `sign(encrypt(compress(plain)))`):
//!
//! ```
//! use co::layer::{LayerExt, Layer};
//! use co::layer::physical::FilesystemBytes;
//! use co::layer::cache::LruCache;
//! use co::layer::compression::ZstdCompressed;
//! use co::layer::privacy::ChaCha20Encrypted;
//! use co::layer::security::Ed25519Signed;
//! use co::proto::sync::CoFile;
//!
//! let dir = tempfile::tempdir().unwrap();
//! let stack = FilesystemBytes::new(dir.path())
//!     .stack(Ed25519Signed::new([7u8; 32]))
//!     .stack(ChaCha20Encrypted::new([9u8; 32]))
//!     .stack(ZstdCompressed::new(3))
//!     .stack(LruCache::new(128));
//!
//! let original = CoFile {
//!     path: "diary/2026-04-27.md".into(),
//!     content: b"# Private thoughts".to_vec(),
//!     sha256: String::new(),
//!     mime_type: "text/markdown".into(),
//!     modified_ns: 0,
//! };
//! stack.write(original.clone()).unwrap();
//!
//! // A read request carries just the addressing metadata.
//! let key = CoFile { content: Vec::new(), ..original.clone() };
//! let got = stack.read(key).unwrap();
//! assert_eq!(got.content, original.content);
//! ```

use std::fmt;

pub mod cache;
pub mod compression;
pub mod measured;
pub mod physical;
pub mod privacy;
pub mod security;
pub mod storage;
pub mod tee;
pub mod transport;

/// A layer transforms a `CoFile` going one direction (read or write) and the
/// reverse on the other. Every layer is also a sink/source for the layer above
/// it. Errors propagate; a failure at any layer aborts the whole flow.
pub trait Layer: Send + Sync {
    type ReadInput;
    type ReadOutput;
    type WriteInput;
    type WriteOutput;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Read direction: bottom → top.
    fn read(&self, input: Self::ReadInput) -> Result<Self::ReadOutput, Self::Error>;

    /// Write direction: top → bottom.
    fn write(&self, input: Self::WriteInput) -> Result<Self::WriteOutput, Self::Error>;

    /// Human-readable layer name used in [`StackError`] diagnostics.
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Stamps the four `CoFile`-in/`CoFile`-out associated types — the v1 "all
/// `CoFile`" model shared by every concrete layer in this module.
macro_rules! cofile_io {
    () => {
        type ReadInput = $crate::proto::sync::CoFile;
        type ReadOutput = $crate::proto::sync::CoFile;
        type WriteInput = $crate::proto::sync::CoFile;
        type WriteOutput = $crate::proto::sync::CoFile;
    };
}
pub(crate) use cofile_io;

/// Composer: stacks two layers so the bottom's output feeds the top's input on
/// reads, and the top's output feeds the bottom's input on writes.
pub struct Stack<Bottom, Top> {
    bottom: Bottom,
    top: Top,
}

impl<B, T> Stack<B, T>
where
    B: Layer,
    T: Layer<ReadInput = B::ReadOutput, WriteOutput = B::WriteInput>,
{
    pub fn new(bottom: B, top: T) -> Self {
        Self { bottom, top }
    }

    /// Borrow the bottom (lower) layer — useful for inspecting a terminal's
    /// state (e.g. a cache's contents) after composing.
    pub fn bottom(&self) -> &B {
        &self.bottom
    }

    /// Borrow the top (upper) layer.
    pub fn top(&self) -> &T {
        &self.top
    }
}

impl<B, T> Layer for Stack<B, T>
where
    B: Layer,
    T: Layer<ReadInput = B::ReadOutput, WriteOutput = B::WriteInput>,
{
    type ReadInput = B::ReadInput;
    type ReadOutput = T::ReadOutput;
    type WriteInput = T::WriteInput;
    type WriteOutput = B::WriteOutput;
    type Error = StackError;

    fn read(&self, input: Self::ReadInput) -> Result<Self::ReadOutput, Self::Error> {
        let mid = self
            .bottom
            .read(input)
            .map_err(|e| StackError::new(self.bottom.name(), e))?;
        self.top
            .read(mid)
            .map_err(|e| StackError::new(self.top.name(), e))
    }

    fn write(&self, input: Self::WriteInput) -> Result<Self::WriteOutput, Self::Error> {
        let mid = self
            .top
            .write(input)
            .map_err(|e| StackError::new(self.top.name(), e))?;
        self.bottom
            .write(mid)
            .map_err(|e| StackError::new(self.bottom.name(), e))
    }
}

/// Fluent builder: `bottom.stack(top)` → `Stack { bottom, top }`.
pub trait LayerExt: Layer + Sized {
    fn stack<T>(self, top: T) -> Stack<Self, T>
    where
        T: Layer<ReadInput = Self::ReadOutput, WriteOutput = Self::WriteInput>,
    {
        Stack::new(self, top)
    }
}

impl<L: Layer> LayerExt for L {}

/// Marker traits for canonical layer roles. A stack that ends with a [`Source`]
/// can produce a `CoFile` from no input; a stack ending with a [`Sink`] writes
/// the `CoFile` into the void (storage / network / disk).
pub trait Source: Layer<ReadInput = ()> {}
pub trait Sink: Layer<WriteOutput = ()> {}

/// Error from a composed [`Stack`]: carries `(layer_name, source_error)` for
/// diagnostics. No silent fallbacks — failures propagate and name the layer
/// that produced them.
#[derive(Debug)]
pub struct StackError {
    pub layer: String,
    pub source: Box<dyn std::error::Error + Send + Sync>,
}

impl StackError {
    pub fn new(
        layer: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            layer: layer.into(),
            source: Box::new(source),
        }
    }
}

impl fmt::Display for StackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "layer `{}` failed: {}", self.layer, self.source)
    }
}

impl std::error::Error for StackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// A trivial [`std::error::Error`] used by infallible / simple layers.
#[derive(Debug, thiserror::Error)]
pub enum LayerError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("encoding: {0}")]
    Encoding(String),
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("backend: {0}")]
    Backend(String),
}

#[cfg(test)]
mod tests;
