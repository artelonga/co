//! Storage layers — indexed, durable homes for a `CoFile`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use super::physical::sha256_hex;
use super::physical::write_atomic;
use super::{Layer, LayerError, cofile_io};
use crate::proto::sync::CoFile;

/// Long-term storage in SQLite, keyed by `(path)`. Maps `CoFile.path` →
/// `CoFile` bytes (plus metadata) in a single table. The connection is shared
/// (`Arc<Mutex<_>>`) so the layer is `Send + Sync`.
///
/// Pairs with CO-77 (per-universe SQLite): a real deployment hands in the
/// universe's connection; tests use `Connection::open_in_memory()`.
pub struct SqliteStorage {
    pub conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Wrap an existing connection, ensuring the backing table exists.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Result<Self, LayerError> {
        {
            let c = conn.lock().unwrap();
            c.execute_batch(
                "CREATE TABLE IF NOT EXISTS cofiles (
                    path        TEXT PRIMARY KEY,
                    content     BLOB NOT NULL,
                    sha256      TEXT NOT NULL,
                    mime_type   TEXT NOT NULL,
                    modified_ns INTEGER NOT NULL
                 );",
            )
            .map_err(|e| LayerError::Backend(e.to_string()))?;
        }
        Ok(Self { conn })
    }

    /// Convenience: an in-memory store (used in tests).
    pub fn in_memory() -> Result<Self, LayerError> {
        let conn = Connection::open_in_memory().map_err(|e| LayerError::Backend(e.to_string()))?;
        Self::new(Arc::new(Mutex::new(conn)))
    }
}

impl Layer for SqliteStorage {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, input: CoFile) -> Result<CoFile, LayerError> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c
            .prepare("SELECT content, sha256, mime_type, modified_ns FROM cofiles WHERE path = ?1")
            .map_err(|e| LayerError::Backend(e.to_string()))?;
        let row = stmt
            .query_row([&input.path], |row| {
                Ok(CoFile {
                    path: input.path.clone(),
                    content: row.get::<_, Vec<u8>>(0)?,
                    sha256: row.get::<_, String>(1)?,
                    mime_type: row.get::<_, String>(2)?,
                    modified_ns: row.get::<_, i64>(3)?,
                })
            })
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => LayerError::NotFound(input.path.clone()),
                other => LayerError::Backend(other.to_string()),
            })?;
        Ok(row)
    }

    fn write(&self, input: CoFile) -> Result<CoFile, LayerError> {
        let c = self.conn.lock().unwrap();
        c.execute(
            "INSERT INTO cofiles (path, content, sha256, mime_type, modified_ns)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
                content = excluded.content,
                sha256 = excluded.sha256,
                mime_type = excluded.mime_type,
                modified_ns = excluded.modified_ns",
            rusqlite::params![
                input.path,
                input.content,
                input.sha256,
                input.mime_type,
                input.modified_ns,
            ],
        )
        .map_err(|e| LayerError::Backend(e.to_string()))?;
        Ok(input)
    }
}

/// Content-addressed blob storage on the filesystem (CO-81). The key is the
/// SHA-256 of `content`, so identical bodies dedupe to one object. Used for
/// attachments and large bodies.
///
/// * **write** computes `sha256(content)`, writes `<root>/<sha>` (if absent),
///   and stamps the digest back onto `CoFile.sha256`.
/// * **read** loads `<root>/<request.sha256>`.
pub struct BlobStore {
    pub root: PathBuf,
}

impl BlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, LayerError> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }
}

impl Layer for BlobStore {
    cofile_io!();
    type Error = LayerError;

    fn read(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        if input.sha256.is_empty() {
            return Err(LayerError::Backend(
                "BlobStore read requires sha256 (content address)".into(),
            ));
        }
        let path = self.root.join(&input.sha256);
        let content = std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                LayerError::NotFound(input.sha256.clone())
            } else {
                LayerError::Io(e)
            }
        })?;
        input.content = content;
        Ok(input)
    }

    fn write(&self, mut input: CoFile) -> Result<CoFile, LayerError> {
        let sha = sha256_hex(&input.content);
        let path = self.root.join(&sha);
        if !path.exists() {
            write_atomic(&path, &input.content)?;
        }
        input.sha256 = sha;
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cofile(path: &str, body: &[u8]) -> CoFile {
        CoFile {
            path: path.into(),
            content: body.to_vec(),
            sha256: String::new(),
            mime_type: "text/markdown".into(),
            modified_ns: 5,
        }
    }

    #[test]
    fn sqlite_round_trips_and_upserts() {
        let store = SqliteStorage::in_memory().unwrap();
        let original = cofile("notes/a.md", b"# hi");
        store.write(original.clone()).unwrap();

        let key = CoFile {
            content: Vec::new(),
            sha256: String::new(),
            ..original.clone()
        };
        let got = store.read(key.clone()).unwrap();
        assert_eq!(got.content, original.content);
        assert_eq!(got.mime_type, "text/markdown");

        // Upsert overwrites.
        store.write(cofile("notes/a.md", b"# bye")).unwrap();
        assert_eq!(store.read(key).unwrap().content, b"# bye");
    }

    #[test]
    fn sqlite_missing_is_not_found() {
        let store = SqliteStorage::in_memory().unwrap();
        let err = store.read(cofile("nope.md", b"")).unwrap_err();
        assert!(matches!(err, LayerError::NotFound(_)));
    }

    #[test]
    fn blobstore_is_content_addressed() {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path()).unwrap();
        let written = store.write(cofile("ignored.md", b"blobby")).unwrap();
        assert_eq!(written.sha256, sha256_hex(b"blobby"));

        // read by sha
        let key = CoFile {
            content: Vec::new(),
            sha256: written.sha256.clone(),
            ..cofile("ignored.md", b"")
        };
        assert_eq!(store.read(key).unwrap().content, b"blobby");

        // identical bodies dedupe to the same object
        let again = store.write(cofile("other.md", b"blobby")).unwrap();
        assert_eq!(again.sha256, written.sha256);
    }
}
