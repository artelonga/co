## CO-78 — Hotfix: migration v088 must create the `jobs` table, not assume it

The 3.15.0 prod deploy crash-looped at boot: migration **v088** ran
`ALTER TABLE jobs ADD COLUMN timeout_secs` but `jobs` did not exist on the
production DB (`no such table: jobs`), and the CO-446 guard aborted boot.

Root cause: the base `jobs` CREATE lives in v025 (CO-72) inside an
`if current_version < 25` guard. Any DB already past v25 — i.e. production —
never re-runs it, so the table was never created there. A fresh test DB runs
v25 from scratch and has the table, which hid the gap through CI.

v088 now creates the `jobs` base table + its base indexes with
`CREATE TABLE IF NOT EXISTS` before altering it, making the migration
self-contained on any DB (no-op where the table already exists). Adds a
regression test that drops `jobs` and re-runs v088.

### Why
A migration step must never assume schema produced by an *earlier* version's
guarded block — production may have advanced past that guard before the schema
was added. Same failure class as the v51 `remote_url` ordering fix in this wave.
