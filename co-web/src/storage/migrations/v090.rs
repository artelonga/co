use super::super::Storage;
use crate::storage::schema::ensure_column;

impl Storage {
    /// CO-81: object-storage garbage-collection cursor on `blob_refs`.
    ///
    /// The S3 [`StorageBackend`](crate::storage::backend::s3::S3Backend) does
    /// **not** physically delete a remote object the moment its refcount hits
    /// zero — a network round-trip per drop is wasteful and, with
    /// content-addressed dedupe, the same bytes are frequently re-`put` shortly
    /// after. Instead `delete` decrements the refcount and, at zero, stamps
    /// `unreferenced_at`; a re-`put` clears it again. The GC sweep
    /// (`gc_unreferenced`) reclaims rows whose `unreferenced_at` is older than
    /// the 30-day grace period.
    ///
    /// `NULL` ⇒ the blob is still referenced (the common case, and the value
    /// for every row the local backend writes). Additive + idempotent
    /// (`ensure_column` is a guarded `ALTER TABLE`), never folded into the base
    /// `blob_refs` create in v086, per the migration-version-claim protocol.
    ///
    /// Migration version **v090** (train tip v089; claimed as max + 1).
    pub(super) fn migrate_v090(&mut self, current_version: i64) {
        if current_version < 90 {
            // RFC3339 timestamp of the moment the refcount last dropped to zero.
            // NULL while referenced; set on the zero-drop, cleared on re-put.
            ensure_column(&self.conn, "blob_refs", "unreferenced_at", "TEXT")
                .expect("migration v90: blob_refs.unreferenced_at");

            crate::record_migration!(
                self.conn,
                90,
                "CO-81: blob_refs.unreferenced_at (object-storage GC grace-period cursor)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::Storage;

    #[test]
    fn unreferenced_at_column_exists_and_defaults_null() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(dir.path());

        storage
            .conn()
            .execute(
                "INSERT INTO blob_refs (hash, backend, size, content_type, refcount, created_at) \
                 VALUES ('cafef00d', 's3', 7, 'text/plain', 1, 'now')",
                [],
            )
            .unwrap();

        let unref: Option<String> = storage
            .conn()
            .query_row(
                "SELECT unreferenced_at FROM blob_refs WHERE hash = 'cafef00d'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unref, None, "unreferenced_at defaults to NULL (referenced)");
    }
}
