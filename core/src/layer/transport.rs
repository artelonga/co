//! Network layer — cross-machine transport of a `CoFile`.

use super::physical::{HttpEndpoint, sha256_hex};
use super::{Layer, LayerError, cofile_io};
use crate::proto::sync::CoFile;

/// Vault API transport: reads via `GET /api/v1/universes/:slug/vault/<path>`,
/// writes via `PUT`. Like [`HttpBytes`](super::physical::HttpBytes) it takes an
/// injected [`HttpEndpoint`] so it is sync and unit-testable without a port; the
/// concrete `reqwest` client is wired in the binaries.
pub struct VaultApiTransport {
    pub endpoint: Box<dyn HttpEndpoint>,
    pub slug: String,
}

impl VaultApiTransport {
    pub fn new(endpoint: Box<dyn HttpEndpoint>, slug: impl Into<String>) -> Self {
        Self {
            endpoint,
            slug: slug.into(),
        }
    }

    fn route(&self, path: &str) -> String {
        format!("api/v1/universes/{}/vault/{}", self.slug, path)
    }
}

impl Layer for VaultApiTransport {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, input: CoFile) -> Result<CoFile, LayerError> {
        let content = self.endpoint.get(&self.route(&input.path))?;
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
        self.endpoint
            .put(&self.route(&input.path), &input.content)?;
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

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

    fn cofile(path: &str, body: &[u8]) -> CoFile {
        CoFile {
            path: path.into(),
            content: body.to_vec(),
            sha256: String::new(),
            mime_type: "text/markdown".into(),
            modified_ns: 0,
        }
    }

    #[test]
    fn writes_and_reads_via_vault_route() {
        let endpoint = Box::<MemEndpoint>::default();
        let t = VaultApiTransport::new(endpoint, "yuri-universe");
        let original = cofile("diary/2026-04-27.md", b"protobuf bytes");
        t.write(original.clone()).unwrap();

        let key = CoFile {
            content: Vec::new(),
            ..original.clone()
        };
        let got = t.read(key).unwrap();
        assert_eq!(got.content, original.content);
    }

    #[test]
    fn maps_to_canonical_route() {
        let t = VaultApiTransport::new(Box::<MemEndpoint>::default(), "u");
        assert_eq!(t.route("a/b.md"), "api/v1/universes/u/vault/a/b.md");
    }
}
