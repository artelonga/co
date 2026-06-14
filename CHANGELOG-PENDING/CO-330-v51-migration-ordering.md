## CO-330 — Fix migration v51 ordering: `remote_url` written before the column exists

A 2026-06-14 hotfix (`df79d6a`, "remote_url for quilomboaraucaria content cloning")
added `remote_url=…, remote_ref=…` to the **v51** universe→repo backfill. But those
columns are only added at **v56** (CO-337). On production the column already existed
(the DB was past v56), so the write succeeded — but any *fresh* sequential migration
(UAT reset, anonymous clone, every `Storage::new` in tests) died at v51 with
`no such column: remote_url`, which the CO-446 guard escalates to a `FATAL` process
abort. This surfaced as the `cargo test -p co-web --lib security` binary "exiting
abnormally" (the `pbi_backlogger` tests build a fresh DB), failing the security-audit
gate on every PR.

The fix keeps v51 writing only the columns it owns (`local_repo_path`,
`content_subdirs`) and moves the `remote_url`/`remote_ref` backfill to *after* the
v56 columns are guaranteed to exist, idempotent via `WHERE … remote_url IS NULL`.

### Why
Migration steps must never reference a column added by a later step — a fresh
top-to-bottom migration has to succeed, not just an already-advanced production DB.
