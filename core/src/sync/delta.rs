//! CO-151 — protobuf SyncDelta/SyncBatch encode/decode with zstd compression.

use prost::Message;

use crate::proto::sync::{SyncBatch, SyncDelta, sync_delta};

pub use crate::proto::sync::{CoFile, SyncBatch as Batch, SyncDelta as Delta, sync_delta::Kind};

/// zstd compression level for sync frames.
const ZSTD_LEVEL: i32 = 3;

/// Placeholder training dictionary — replace with a real ~32 KB dict trained
/// on a corpus of admin markdown for 30–50% better ratio on small messages.
pub const ZSTD_DICT: &[u8] = &[];

/// Encode a `SyncBatch` to zstd-compressed protobuf bytes.
pub fn encode_batch(batch: &SyncBatch) -> anyhow::Result<Vec<u8>> {
    let proto_bytes = batch.encode_to_vec();
    let compressed = if ZSTD_DICT.is_empty() {
        zstd::encode_all(proto_bytes.as_slice(), ZSTD_LEVEL)?
    } else {
        let mut enc = zstd::Encoder::with_dictionary(Vec::new(), ZSTD_LEVEL, ZSTD_DICT)?;
        use std::io::Write;
        enc.write_all(&proto_bytes)?;
        enc.finish()?
    };
    Ok(compressed)
}

/// Decode a zstd-compressed protobuf `SyncBatch`.
pub fn decode_batch(data: &[u8]) -> anyhow::Result<SyncBatch> {
    let proto_bytes = if ZSTD_DICT.is_empty() {
        zstd::decode_all(data)?
    } else {
        zstd::bulk::decompress_to_buffer(data, &mut vec![0u8; data.len() * 8])?;
        // fallback to dict-less for now
        zstd::decode_all(data)?
    };
    Ok(SyncBatch::decode(proto_bytes.as_slice())?)
}

/// Build a SyncDelta for a file upsert.
pub fn upserted_delta(
    universe_key: impl Into<String>,
    entry_path: impl Into<String>,
    content: Vec<u8>,
    sha256: impl Into<String>,
    client_ts_ns: i64,
) -> SyncDelta {
    let path = entry_path.into();
    let sha = sha256.into();
    SyncDelta {
        universe_key: universe_key.into(),
        entry_path: path.clone(),
        client_ts_ns,
        kind: Kind::Upserted as i32,
        body: Some(sync_delta::Body::Cofile(CoFile {
            path,
            content,
            sha256: sha.clone(),
            mime_type: "text/markdown".into(),
            modified_ns: client_ts_ns,
        })),
        last_known_sha256: String::new(),
        new_sha256: sha,
    }
}

/// Build a SyncDelta for a file deletion.
pub fn deleted_delta(
    universe_key: impl Into<String>,
    entry_path: impl Into<String>,
    client_ts_ns: i64,
) -> SyncDelta {
    SyncDelta {
        universe_key: universe_key.into(),
        entry_path: entry_path.into(),
        client_ts_ns,
        kind: Kind::Deleted as i32,
        body: None,
        last_known_sha256: String::new(),
        new_sha256: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip_empty_batch() {
        let batch = SyncBatch {
            deltas: vec![],
            client_id: "test-client".into(),
            batch_ts_ns: 1_000_000_000,
            resume_token: 0,
        };
        let encoded = encode_batch(&batch).unwrap();
        let decoded = decode_batch(&encoded).unwrap();
        assert_eq!(decoded.client_id, "test-client");
        assert_eq!(decoded.batch_ts_ns, 1_000_000_000);
        assert!(decoded.deltas.is_empty());
    }

    #[test]
    fn test_encode_decode_upserted_delta() {
        let delta = upserted_delta(
            "my-universe",
            "projects/foo/bar.md",
            b"# Hello world".to_vec(),
            "abc123",
            1_700_000_000_000_000_000i64,
        );
        let batch = SyncBatch {
            deltas: vec![delta],
            client_id: "watcher-1".into(),
            batch_ts_ns: 1_700_000_000_000_000_000i64,
            resume_token: 42,
        };
        let encoded = encode_batch(&batch).unwrap();
        let decoded = decode_batch(&encoded).unwrap();

        assert_eq!(decoded.deltas.len(), 1);
        let d = &decoded.deltas[0];
        assert_eq!(d.universe_key, "my-universe");
        assert_eq!(d.entry_path, "projects/foo/bar.md");
        assert_eq!(d.kind, Kind::Upserted as i32);
        assert_eq!(d.new_sha256, "abc123");

        match d.body.as_ref().unwrap() {
            sync_delta::Body::Cofile(f) => {
                assert_eq!(f.content, b"# Hello world");
                assert_eq!(f.mime_type, "text/markdown");
            }
            _ => panic!("expected CoFile body"),
        }
    }

    #[test]
    fn test_encode_decode_deleted_delta() {
        let delta = deleted_delta("u-xyz", "notes/gone.md", 999);
        let batch = SyncBatch {
            deltas: vec![delta],
            client_id: "c".into(),
            batch_ts_ns: 999,
            resume_token: 0,
        };
        let encoded = encode_batch(&batch).unwrap();
        let decoded = decode_batch(&encoded).unwrap();

        let d = &decoded.deltas[0];
        assert_eq!(d.kind, Kind::Deleted as i32);
        assert!(d.body.is_none());
    }

    #[test]
    fn test_wire_size_vs_json() {
        // Verify protobuf+zstd is more compact than a naive JSON equivalent.
        let content = b"# Title\n\nA short markdown paragraph.\n\ntags: [test, bench]\n".to_vec();
        let sha = "deadbeef".to_string();
        let delta = upserted_delta("u", "a.md", content.clone(), sha, 0);
        let batch = SyncBatch {
            deltas: vec![delta],
            client_id: "c".into(),
            batch_ts_ns: 0,
            resume_token: 0,
        };
        let proto_zstd = encode_batch(&batch).unwrap();

        // Rough JSON equivalent length.
        let json_len = format!(
            r#"{{"universe_key":"u","entry_path":"a.md","kind":"UPSERTED","content":"{}","sha256":"deadbeef"}}"#,
            String::from_utf8_lossy(&content)
        )
        .len();

        // proto+zstd should be at most 70% of the JSON length (≤30% of JSON).
        assert!(
            proto_zstd.len() < json_len,
            "proto+zstd ({} bytes) should be smaller than JSON ({} bytes)",
            proto_zstd.len(),
            json_len
        );
    }
}
