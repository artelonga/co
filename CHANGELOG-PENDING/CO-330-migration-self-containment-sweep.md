## CO-330 — Migration self-containment sweep (post-outage hardening)

Follow-up to the 3.15.0 migration-ordering fixes (v51/`remote_url`, v088/`jobs`).
Audited every meta-DB and per-universe migration for the same bug class — a step
that references schema created only inside an earlier `if current_version < N`
guard, which production passed long ago (so it never got it, while fresh-DB CI
hides the gap). No further *live* breakage was found, but two structurally
fragile spots — one retroactive guard edit away from a repeat — are now hardened:

- **v085** `workspace_states.scope`: the table's only CREATE is in v70's guard;
  `migrate_v085` now `CREATE TABLE IF NOT EXISTS workspace_states` before
  `ensure_column`.
- **v030** `subscriptions.pinned_state`: same shape vs v20's CREATE; v30 now
  recreates the table defensively first.

Both gain a regression test that drops the table and re-runs the migration. Also
hardened `scripts/pipeline-deploy-gate.sh`: it no longer requires a UAT pipeline
report (UAT is decommissioned) — it gates on the local (+ optional prod-smoke)
report, so the documented "real gate" actually passes.

### Why
A migration must be self-contained: a fresh top-to-bottom run must succeed, not
just an already-advanced prod DB. `ensure_column` is column-safe but ALTERs a
missing *table* — so the base table must be ensured in the same block.
