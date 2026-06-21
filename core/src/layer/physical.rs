//! Physical layer — raw bytes on disk or over HTTP.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{Layer, LayerError, cofile_io};
use crate::proto::sync::CoFile;

/// Hex-encode the SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Physical: bytes on a local filesystem, addressed by the `CoFile.path`
/// (relative to `root`).
///
/// * **write** persists `cofile.content` atomically (write to a temp sibling,
///   then `rename`), creating parent directories as needed. Metadata fields
///   (mime/modified/sha) are *not* persisted by this raw-byte layer — they
///   round-trip via the read request.
/// * **read** loads the bytes at `root/<request.path>` into `content`, echoing
///   the request's addressing metadata. `sha256` is recomputed only when the
///   request left it empty.
pub struct FilesystemBytes {
    pub root: PathBuf,
}

impl FilesystemBytes {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn full(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }
}

impl Layer for FilesystemBytes {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, input: CoFile) -> Result<CoFile, LayerError> {
        let full = self.full(&input.path);
        let content = std::fs::read(&full).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                LayerError::NotFound(input.path.clone())
            } else {
                LayerError::Io(e)
            }
        })?;
        let sha256 = if input.sha256.is_empty() {
            sha256_hex(&content)
        } else {
            input.sha256
        };
        Ok(CoFile {
            path: input.path,
            content,
            sha256,
            mime_type: input.mime_type,
            modified_ns: input.modified_ns,
        })
    }

    fn write(&self, input: CoFile) -> Result<CoFile, LayerError> {
        let full = self.full(&input.path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&full, &input.content)?;
        Ok(input)
    }
}

/// Atomic write: temp sibling + rename. Mirrors the `put_vault_file` raw-write
/// behavior so the pilot refactor is behavior-identical.
pub(crate) fn write_atomic(full: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = full.with_extension("co-tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, full)
}

/// A minimal HTTP endpoint abstraction so [`HttpBytes`] (and
/// [`VaultApiTransport`](super::transport::VaultApiTransport)) stay sync and
/// testable without binding a real port. The concrete `reqwest`-backed impl
/// lives in the binaries (co-cli/co-web), which already depend on reqwest.
pub trait HttpEndpoint: Send + Sync {
    /// `GET <base>/<path>` → body bytes.
    fn get(&self, path: &str) -> Result<Vec<u8>, LayerError>;
    /// `PUT <base>/<path>` with `body`.
    fn put(&self, path: &str, body: &[u8]) -> Result<(), LayerError>;
}

/// Bearer auth token carried by an [`HttpBytes`] layer.
#[derive(Clone, Default)]
pub struct AuthBearer(pub String);

/// Physical: bytes over HTTP. Reads via GET, writes via PUT, addressed by
/// `CoFile.path`. The transport is injected so this layer is unit-testable
/// against an in-memory endpoint.
pub struct HttpBytes {
    pub endpoint: Box<dyn HttpEndpoint>,
    pub auth: AuthBearer,
}

impl HttpBytes {
    pub fn new(endpoint: Box<dyn HttpEndpoint>, auth: AuthBearer) -> Self {
        Self { endpoint, auth }
    }
}

impl Layer for HttpBytes {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, input: CoFile) -> Result<CoFile, LayerError> {
        let content = self.endpoint.get(&input.path)?;
        let sha256 = if input.sha256.is_empty() {
            sha256_hex(&content)
        } else {
            input.sha256
        };
        Ok(CoFile {
            path: input.path,
            content,
            sha256,
            mime_type: input.mime_type,
            modified_ns: input.modified_ns,
        })
    }

    fn write(&self, input: CoFile) -> Result<CoFile, LayerError> {
        self.endpoint.put(&input.path, &input.content)?;
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn cofile(path: &str, body: &[u8]) -> CoFile {
        CoFile {
            path: path.into(),
            content: body.to_vec(),
            sha256: String::new(),
            mime_type: "text/markdown".into(),
            modified_ns: 42,
        }
    }

    #[test]
    fn filesystem_round_trips_content() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FilesystemBytes::new(dir.path());
        let original = cofile("notes/a.md", b"# hello");
        fs.write(original.clone()).unwrap();

        let key = CoFile {
            content: Vec::new(),
            ..original.clone()
        };
        let got = fs.read(key).unwrap();
        assert_eq!(got.content, original.content);
        assert_eq!(got.mime_type, "text/markdown");
        assert_eq!(got.sha256, sha256_hex(&original.content));
    }

    #[test]
    fn filesystem_creates_nested_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FilesystemBytes::new(dir.path());
        fs.write(cofile("deep/nested/path/x.md", b"x")).unwrap();
        assert!(dir.path().join("deep/nested/path/x.md").exists());
    }

    #[test]
    fn filesystem_read_missing_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let fs = FilesystemBytes::new(dir.path());
        let err = fs.read(cofile("nope.md", b"")).unwrap_err();
        assert!(matches!(err, LayerError::NotFound(_)));
    }

    #[derive(Default)]
    struct MemEndpoint(Mutex<HashMap<String, Vec<u8>>>);
    impl HttpEndpoint for MemEndpoint {
        fn get(&self, path: &str) -> Result<Vec<u8>, LayerError> {
            self.0
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| LayerError::NotFound(path.into()))
        }
        fn put(&self, path: &str, body: &[u8]) -> Result<(), LayerError> {
            self.0.lock().unwrap().insert(path.into(), body.to_vec());
            Ok(())
        }
    }

    #[test]
    fn http_round_trips_via_injected_endpoint() {
        let http = HttpBytes::new(Box::<MemEndpoint>::default(), AuthBearer("t".into()));
        let original = cofile("u/v.md", b"payload");
        http.write(original.clone()).unwrap();
        let key = CoFile {
            content: Vec::new(),
            ..original.clone()
        };
        let got = http.read(key).unwrap();
        assert_eq!(got.content, original.content);
    }
}
