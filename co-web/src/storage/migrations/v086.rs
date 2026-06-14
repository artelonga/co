use super::super::Storage;

impl Storage {
    /// CO-458: `blob_refs` — the central ledger for the pluggable
    /// [`StorageBackend`](crate::storage::backend::StorageBackend) (StaaS keystone).
    ///
    /// Every content-addressed blob written through a backend gets one row here,
    /// keyed by its `hash` (sha256 hex). The row carries:
    ///   · `backend`      — which impl stored it (`local`, later `s3`/partner).
    ///   · `size`         — byte length (for StaaS billing + quota accounting).
    ///   · `content_type` — MIME of the stored bytes.
    ///   · `refcount`     — how many logical references point at this blob, so a
    ///                      re-`put` of identical bytes is a refcount bump (dedupe)
    ///                      and a `delete` only removes the file when it hits zero.
    ///   · `created_at`   — RFC3339 timestamp of first insert.
    ///
    /// This table is the single point where S3 (CO-81), partner backends, and
    /// StaaS billing plug in without touching call-sites. v085 was the prior tip
    /// (`workspace_states.scope`); this task claims **v86**.
    ///
    /// Idempotent: `CREATE TABLE IF NOT EXISTS` so re-running the chain on an
    /// existing DB is a no-op.
    pub(super) fn migrate_v086(&mut self, current_version: i64) {
        if current_version < 86 {
            self.conn
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS blob_refs (
                        hash         TEXT PRIMARY KEY,
                        backend      TEXT NOT NULL,
                        size         INTEGER NOT NULL,
                        content_type TEXT NOT NULL,
                        refcount     INTEGER NOT NULL DEFAULT 1,
                        created_at   TEXT NOT NULL
                    );",
                )
                .expect("migration v86: blob_refs table");

            crate::record_migration!(
                self.conn,
                86,
                "CO-458: blob_refs (content-addressed StorageBackend ledger + refcount dedupe)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn blob_refs_table_exists_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        storage
            .conn()
            .execute(
                "INSERT INTO blob_refs (hash, backend, size, content_type, refcount, created_at) \
                 VALUES ('deadbeef', 'local', 12, 'text/plain', 1, 'now')",
                [],
            )
            .unwrap();

        let (backend, size, refcount): (String, i64, i64) = storage
            .conn()
            .query_row(
                "SELECT backend, size, refcount FROM blob_refs WHERE hash = 'deadbeef'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(backend, "local");
        assert_eq!(size, 12);
        assert_eq!(refcount, 1);
    }
}
