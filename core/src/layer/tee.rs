//! `Tee` — fan-out writer / fall-back reader.

use super::{Layer, LayerError, cofile_io};
use crate::proto::sync::CoFile;

/// Writes to **both** layers; reads prefer `primary` and fall back to
/// `secondary`. Used for LiteFS replication (write primary + replicas) or
/// multi-tier storage (SQLite + S3 blob mirror).
///
/// Both layers must be `CoFile → CoFile` (the v1 model). On write, `primary`
/// runs first; if it succeeds, `secondary` runs too and the **primary's** output
/// is returned (the secondary is a mirror). On read, `primary` is tried; on
/// error, `secondary` is tried; the first success wins.
pub struct Tee<P, S> {
    primary: P,
    secondary: S,
}

impl<P, S> Tee<P, S> {
    pub fn new(primary: P, secondary: S) -> Self {
        Self { primary, secondary }
    }
}

impl<P, S> Layer for Tee<P, S>
where
    P: Layer<ReadInput = CoFile, ReadOutput = CoFile, WriteInput = CoFile, WriteOutput = CoFile>,
    S: Layer<ReadInput = CoFile, ReadOutput = CoFile, WriteInput = CoFile, WriteOutput = CoFile>,
{
    cofile_io!();
    type Error = LayerError;

    fn read(&self, input: CoFile) -> Result<CoFile, LayerError> {
        match self.primary.read(input.clone()) {
            Ok(v) => Ok(v),
            Err(primary_err) => self.secondary.read(input).map_err(|secondary_err| {
                LayerError::Backend(format!(
                    "tee read failed on both: primary={primary_err}; secondary={secondary_err}"
                ))
            }),
        }
    }

    fn write(&self, input: CoFile) -> Result<CoFile, LayerError> {
        let out = self
            .primary
            .write(input.clone())
            .map_err(|e| LayerError::Backend(format!("tee primary write: {e}")))?;
        self.secondary
            .write(input)
            .map_err(|e| LayerError::Backend(format!("tee secondary write: {e}")))?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::storage::SqliteStorage;

    fn cofile(path: &str, body: &[u8]) -> CoFile {
        CoFile {
            path: path.into(),
            content: body.to_vec(),
            sha256: "s".into(),
            mime_type: "text/markdown".into(),
            modified_ns: 0,
        }
    }

    #[test]
    fn write_fans_out_to_both() {
        let primary = SqliteStorage::in_memory().unwrap();
        let secondary = SqliteStorage::in_memory().unwrap();
        let tee = Tee::new(primary, secondary);
        let original = cofile("a.md", b"replicated");
        tee.write(original.clone()).unwrap();

        let key = CoFile {
            content: Vec::new(),
            ..original.clone()
        };
        // Both backings hold the value.
        assert_eq!(tee.read(key.clone()).unwrap().content, b"replicated");
        assert_eq!(tee.secondary.read(key).unwrap().content, b"replicated");
    }

    #[test]
    fn read_falls_back_to_secondary() {
        let primary = SqliteStorage::in_memory().unwrap();
        let secondary = SqliteStorage::in_memory().unwrap();
        // Only the secondary has the value.
        secondary.write(cofile("only.md", b"backup")).unwrap();
        let tee = Tee::new(primary, secondary);
        let key = cofile("only.md", b"");
        let key = CoFile {
            content: Vec::new(),
            sha256: String::new(),
            ..key
        };
        assert_eq!(tee.read(key).unwrap().content, b"backup");
    }
}
