# Sprint -3 (2026-04-17 → 2026-04-30)

**Sprint Goal**: (retrospective — inferred from PBIs)
**Release**: (no release in this window)
**Velocity**: 21 PBIs delivered

## Delivered PBIs

### CO-61 — Sync Protocol v1 — op log + content-addressed blobs + 3-way merge + recursive resolution (#12)
_Merged: 2026-04-30_

_(no acceptance criteria in spec)_

### CO-72 — Doc-generator hooks — scaladoc, sphinx, mkdocs, redoc, rustdoc, jsdoc (#11)
_Merged: 2026-04-30_

_(no acceptance criteria in spec)_

### CO-71 — Per-universe schema validator + generic JSON entry storage (#10)
_Merged: 2026-04-30_

_(no acceptance criteria in spec)_

### CO-70 — Manifest format spec — _universe.yaml at universe root (#9)
_Merged: 2026-04-30_

_(no acceptance criteria in spec)_

### CO-140 — co-dev universe row — seed on startup so it appears in Yuri's sidebar
_Merged: 2026-04-30_

- [x] `co-dev` row exists in `universes` table after deploy
- [x] Yuri's sidebar lists CO Dev after next login
- [x] `GET /api/v1/universes/co-dev` still served by `dev_board.rs` (takes route priority)

### CO-64 — Post-GitHub cleanup — remove dead git_sync.rs, formalize ARCHITECTURE.md (#8)
_Merged: 2026-04-30_

_(no acceptance criteria in spec)_

### CO-119 — Restore-drill script + quarterly cadence + result log (#7)
_Merged: 2026-04-30_

- [ ] `tools/restore-drill.sh` exists, executable, idempotent
- [ ] Scratch-app naming uses UTC timestamp; never collides
- [ ] Tear-down runs in `trap` so a failure mid-drill still destroys the scratch app
- [ ] One manual run logged success in `tools/restore-drill.log`
- [ ] Quarterly cron scheduled (Fly Machine cron, or `schedule` agent — owner's call)
- [ ] `docs/OPERATIONS.md` "Backup & restore" section links to this script and notes the quarterly cadence

### CO-117 — Cloudflare CDN in front of co.artelonga.com.br (cache rules + auth-cookie passthrough) (#6)
_Merged: 2026-04-30_

- [ ] DNS migrated to Cloudflare with proxy enabled
- [ ] Cache rules applied per spec, verified with `curl -I` for each path class
- [ ] Auth flow (UAT login on prod): `password-login` returns `Set-Cookie` and is not served from cache on subsequent identical requests from another IP
- [ ] No Fly origin changes required (zero risk to current deploys)
- [ ] Synthetic load test from 5 geographies shows ≥40% reduction in p50 TTFB on static assets
- [ ] Documented in `docs/OPERATIONS.md` under "Edge / CDN"

### CO-138 — Wave 2 Playwright e2e coverage — sidebar trio, mermaid svg, onboarding banner (#5)
_Merged: 2026-04-30_

- [x] All three test files committed under `co-web/e2e/wave-2/`
- [x] `BASE_URL=https://co-artelonga-uat.fly.dev npx playwright test e2e/wave-2/` exits 0
- [x] Tests are added to the post-deploy gate in `docs/OPERATIONS.md`
- [x] `loginAsAdmin` fixture documented (existing or new helper) for any future test that needs auth

### CO-137 — Investigate why migration v22 didn't apply on prod + harden ALTER ADD COLUMN against partial-application (#4)
_Merged: 2026-04-30_

- [ ] Diagnostic endpoint or script confirms exact prod schema state for `universes` (column list + schema_version rows)
- [ ] Root cause documented — written into `feedback_migration_column_reads.md` + this ticket's resolution
- [ ] `ensure_column` helper exists in `co-web/src/storage.rs` and is unit-tested
- [ ] v22 (and any future column-add migration) uses `ensure_column`
- [ ] Backfill: prod's universes table has `parent_key` column after this ticket ships, and the timeline trio shows `parent_key="template"` in `GET /api/v1/universes/tempo`
- [ ] Quilombo + yggdrasil seeders no longer log `Seeding …` on every boot (confirms `*_exists` checks are working again)

### CO-103 — Per-deploy regression smoke test — scripts/smoke-{prod,uat}.sh
_Merged: 2026-04-30_

- [ ] Two scripts at `scripts/smoke-prod.sh` and `scripts/smoke-uat.sh` plus the shared `scripts/smoke-lib.sh`.
- [ ] Each is executable (`chmod +x`).
- [ ] Running against current prod (1.21.1) exits 0.
- [ ] Running against a deliberately broken target (e.g. wrong `BASE_URL`) exits 1 with clear failure lines.
- [ ] At least one of the checks (#4, the event count pin) is verified to actually fire — author runs the script, deletes one event from the local seed, builds, deploys to UAT, runs `smoke-uat.sh`, confirms it fails on check 4 with the right message, then restores.
- [ ] `docs/OPERATIONS.md` references this script under "Smoke test post-deploy".

### CO-94 — Obsidian-like vault viewer — folder tree + markdown panel for raw imported content
_Merged: 2026-04-29_

_(no acceptance criteria in spec)_

### CO-93 — Universe types + sync + deployment — unified architecture (public-static / private-static / private-dynamic)
_Merged: 2026-04-28_

_(no acceptance criteria in spec)_

### CO-92 — Unified timeline view — events from any universe with linear+log scrolling
_Merged: 2026-04-28_

_(no acceptance criteria in spec)_

### CO-82 — UAT mirrors prod content on reset — HTTP pull of yuri's universes
_Merged: 2026-04-27_

_(no acceptance criteria in spec)_

### CO-90 — Drop global admin tier — every user owns their universes; tier is billing-only
_Merged: 2026-04-27_

_(no acceptance criteria in spec)_

### CO-85 — Password-login on prod — replace email-code friction with Argon2id auth
_Merged: 2026-04-27_

_(no acceptance criteria in spec)_

### CO-83 — Mermaid.js diagram rendering — C4, ER, flowcharts, sequence, state, class
_Merged: 2026-04-27_

_(no acceptance criteria in spec)_

### CO-84 — Extract co auto into dev/co-auto crate — trait-based composable pipeline
_Merged: 2026-04-26_

_(no acceptance criteria in spec)_

### CO-66 — API hygiene — 500→409 on duplicate key, fix seed description override, no-auto-stop UAT
_Merged: 2026-04-26_

_(no acceptance criteria in spec)_

### CO-65 — Visibility on PUT — let owners flip universe visibility via API
_Merged: 2026-04-26_

_(no acceptance criteria in spec)_

## Carried Over

- (none tracked — retrospective simulation uses merge commits only)
