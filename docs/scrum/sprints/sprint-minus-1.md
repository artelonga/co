# Sprint -1 (2026-05-15 → 2026-05-28)

**Sprint Goal**: (retrospective — inferred from PBIs)
**Release**: v2.31.1
**Velocity**: 76 PBIs delivered

## Delivered PBIs

### CO-314 — (spec not found) (#116)
_Merged: 2026-05-28_
_Release: v2.31.1_

_(no acceptance criteria in spec)_

### CO-313 — (spec not found) (#115)
_Merged: 2026-05-28_
_Release: v2.31.1_

_(no acceptance criteria in spec)_

### CO-311 — (spec not found) (#114)
_Merged: 2026-05-28_
_Release: v2.31.1_

_(no acceptance criteria in spec)_

### CO-310 — (spec not found) (#113)
_Merged: 2026-05-28_
_Release: v2.31.1_

_(no acceptance criteria in spec)_

### CO-298 — CO-284-I — `co serve --staging` mode (latency + fault injection decorators) (#112)
_Merged: 2026-05-28_
_Release: v2.31.1_

- [ ] `co serve --staging` starts a server that demonstrably injects latency (curl `/api/health` is 50-100ms slower than normal)
- [ ] About 5% of blob ops return 503 (verified by hammering an upload endpoint in a loop)
- [ ] All decorator wrapping is opt-out per plug (you can disable any individual one)
- [ ] Normal `co serve` (no `--staging`) is unaffected

### CO-307 — (spec not found) (#110)
_Merged: 2026-05-28_
_Release: v2.31.1_

_(no acceptance criteria in spec)_

### CO-293 — CO-284-D — Cache trait + in-process LRU default impl (#108)
_Merged: 2026-05-28_
_Release: v2.31.1_

- [ ] Trait `Cache` exists in `co-web/src/infra/cache.rs`
- [ ] `LruCache` default impl works identically to current cache layer
- [ ] All call sites go through the trait
- [ ] Tests pass

### CO-294 — CO-284-E — Blob store trait (standardize on top of CO-263's R2 adapter) (#107)
_Merged: 2026-05-28_
_Release: v2.31.1_

- [ ] Trait `BlobStore` exists; both `LocalFsBlobStore` and the R2 adapter implement it
- [ ] Behavior unchanged with default config
- [ ] Setting `CO_BLOB_BACKEND=r2` + R2 credentials switches the backend at boot, no code change
- [ ] Tests pass

### CO-292 — CO-284-C — Worker executor trait (formalize CO-223 with enqueue/cancel) (#106)
_Merged: 2026-05-28_
_Release: v2.31.1_

- [ ] Trait `WorkerExecutor` exists in `co-web/src/infra/workers.rs`
- [ ] `InProcessExecutor` implements it; existing worker behavior unchanged
- [ ] All current Worker impls (embedding, notification, push, deployment_snapshot) work through the trait
- [ ] Tests pass (`cargo test --workspace`)
- [ ] Memory `feedback_no_panic_under_mutex.md` constraint preserved: workers don't panic while holding Mutex<Storage>

### CO-296 — CO-284-G — Auth provider trait (extract JWT flow, prepare for OAuth/SSO) (#103)
_Merged: 2026-05-27_
_Release: v2.31.1_

- [ ] Trait `AuthProvider` exists; `LocalJwtProvider` implements it
- [ ] All current login flows (password, magic-code, UAT) work through the trait
- [ ] No regressions in `cargo test --test auth_*`
- [ ] Adding a hypothetical OAuth provider would touch only `infra/auth.rs`, not route handlers

### CO-300 — CO-284-K — `co::testkit::TestServer` (spawn real co serve instances for integration tests) (#104)
_Merged: 2026-05-27_
_Release: v2.31.1_

- [ ] `co-testkit::TestServer::start()` spawns a real `co serve` in <2s
- [ ] Tests using it pass and exercise the full code path (no mocked middleware)
- [ ] At least 5 existing tests are migrated to TestServer
- [ ] Drop / shutdown cleans up the data dir and the process; no orphan `co` processes after tests run

### CO-306 — (spec not found) (#105)
_Merged: 2026-05-27_
_Release: v2.31.1_

_(no acceptance criteria in spec)_

### CO-282 — Localhost distribution — `co serve` + browser auto-launch + Tauri shell roadmap (#97)
_Merged: 2026-05-27_
_Release: v2.31.1_

- [ ] `co serve` starts a server on `127.0.0.1:54321`, accessible via browser
- [ ] `co serve --open` launches the user's default browser to that URL
- [ ] `co serve --data-dir ~/foo` uses `~/foo` for SQLite + entries
- [ ] `co serve --public` binds `0.0.0.0` and prints a security warning
- [ ] SPA loads fully against localhost — no failed requests, no broken images
- [ ] All major flows work locally: create universe, add entry, drag kanban card, edit profile
- [ ] Ctrl-C shuts down cleanly without corrupting SQLite (verify with `PRAGMA integrity_check`)
- [ ] GitHub releases include binaries for 5 OS/arch combos
- [ ] Install one-liner works on a fresh macOS / Linux box without pre-existing Rust toolchain

### CO-305 — E2e residual failures sweep — 9 specific bugs surfaced by CO-304 (#102)
_Merged: 2026-05-27_
_Release: v2.31.1_

- [ ] All 9 originally-failing tests pass on a clean data dir
- [ ] Total e2e count: 84 passing / 0 failing
- [ ] CI on the PR is green on first try
- [ ] No regressions in the 75 already-passing tests
- [ ] Each fix is the **smallest possible** change at the **right layer** (test bug → test fix; server bug → server fix; data bug → seed fix)

### CO-304 — E2e selector + timing quality pass — eliminate Carregando/timeout brittleness (#101)
_Merged: 2026-05-27_
_Release: v2.31.1_

- [ ] Local e2e run with clean data dir produces 0 failures
- [ ] CI on the resulting PR passes e2e green
- [ ] CO-303's localhost magic-code flow remains exercisable via the new tests (covered by CO-303-B follow-up)
- [ ] No test logic changes — only selectors + timing + setup
- [ ] `e2e/README.md` documents the data-testid + waitForBoardReady patterns

### CO-303 — Local-fidelity auth — inline magic-code display + admin password fallback in SPA login modal (#100)
_Merged: 2026-05-26_
_Release: v2.31.0_

- [ ] Localhost boot + open `http://localhost:3000` + click Entrar + enter `yuri@uat.local` → modal shows the magic code inline → user can complete login through the UI in &lt; 10 seconds, no curl, no log inspection
- [ ] Production deploy: SPA login modal has zero visual changes; no `dev_code` ever returned; no password tab visible (unless admin is seeded)
- [ ] UAT environment: same as localhost but with the admin password tab available (since `CO_ENV=uat` exposes it)
- [ ] CO_SEED_ADMIN_EMAIL + CO_SEED_ADMIN_PASSWORD_HASH work in production to seed an admin user
- [ ] Existing e2e tests still pass (they can keep using uat-login for now; migration to magic-code is a follow-up)

### CO-302 — Test pyramid restructure — parallelize e2e, cut redundancy, add component layer, run locally (#99)
_Merged: 2026-05-26_
_Release: v2.31.0_

_(no acceptance criteria in spec)_

### CO-290 — CO-284-A — Storage trait abstraction + SQLite impl (#98)
_Merged: 2026-05-25_
_Release: v2.31.0_

- [ ] Trait `Storage` exists in `co-web/src/infra/storage.rs`
- [ ] `SqliteStorage` implements it; behavior identical to before
- [ ] At least the entry-reading routes (`/api/v1/universes/{u}/entries`) go through the trait, not direct connection access
- [ ] All existing tests pass (`cargo test --workspace`)
- [ ] No new clippy warnings

### CO-281 — Fly cost audit — rightsize per app, enable auto-suspend, extract embedding sidecar (#94)
_Merged: 2026-05-24_
_Release: v2.30.2_

- [ ] `docs/infra/fly-baseline-2026-05.md` exists with current sizing snapshot
- [ ] All 3 low-traffic apps have `auto_stop_machines = "suspend"` and `min_machines_running = 0`
- [ ] `co-embedding` Fly app exists, suspends when idle, wakes on POST `/embed`
- [ ] `co-artelonga` no longer loads the embedding model in-process — verified by `flyctl ssh console -a co-artelonga -C "ps aux | grep embed"` showing no model thread
- [ ] `co-artelonga` runs on shared-1x 512MB without OOM for 1 week post-Phase-3
- [ ] `/admin/deployments` shows a "Estimated monthly: $X.YY" sum
- [ ] `CLAUDE.md` deployment section updated with new sizing
- [ ] Total measured monthly Fly bill ≤ 60% of pre-CO-281 baseline (target $8-12/mo for machines)

### CO-277 — Recursive subspace addressing — sub-universe task resolution in co-auto
_Merged: 2026-05-23_
_Release: v2.29.0_

_(no acceptance criteria in spec)_

### CO-275 — Agent session events — capture tokens/tools/skills/duration per co-auto run; surface on kanban cards (#93)
_Merged: 2026-05-23_
_Release: v2.29.0_

_(no acceptance criteria in spec)_

### CO-276 — co-auto CLI simplification — positional task arg, smarter defaults, drop redundant flags (#92)
_Merged: 2026-05-23_
_Release: v2.29.0_

_(no acceptance criteria in spec)_

### CO-274 — co-auto context budget — cut from ~150k chars to ~30k via skills + per-universe CLAUDE.md (#91)
_Merged: 2026-05-23_
_Release: v2.29.0_

_(no acceptance criteria in spec)_

### CO-273 — Centralized deployment dashboard — machines + sizes + statuses + versions across all units (#90)
_Merged: 2026-05-23_
_Release: v2.29.0_

_(no acceptance criteria in spec)_

### CO-272 — Kanban view shows entries-as-tasks, not just legacy projects — close the dogfooding gap (#89)
_Merged: 2026-05-23_
_Release: v2.29.0_

_(no acceptance criteria in spec)_

### CO-270 — Items list final fix — audit middleware chain; identify silent-empty wrapper for anonymous (#88)
_Merged: 2026-05-22_
_Release: v2.25.0_

_(no acceptance criteria in spec)_

### CO-271 — Verify CO-269 deploy + complete LICENSE seed (still 404 in prod) (#87)
_Merged: 2026-05-22_
_Release: v2.25.0_

_(no acceptance criteria in spec)_

### CO-268 — List items filter — items SELECT is stricter than COUNT for anonymous (post-CO-266) (#86)
_Merged: 2026-05-22_
_Release: v2.25.0_

_(no acceptance criteria in spec)_

### CO-269 — Seed LICENSE.md into /co universe (currently 404 at /co/license) (#85)
_Merged: 2026-05-22_
_Release: v2.25.0_

_(no acceptance criteria in spec)_

### CO-267 — CO-261 phase B — cross-repo sync (yggdrasil/rfq/qb/artelonga work folders → CO universes) (#84)
_Merged: 2026-05-22_
_Release: v2.25.0_

_(no acceptance criteria in spec)_

### CO-266 — List endpoint visibility — total counts correctly but items array empty for anonymous (#83)
_Merged: 2026-05-22_
_Release: v2.25.0_

_(no acceptance criteria in spec)_

### CO-264 — Universe = recursive folder tree — per-universe CHANGELOG, index.md, README.md at every level; folder-prefix filtering (#82)
_Merged: 2026-05-22_
_Release: v2.25.0_

_(no acceptance criteria in spec)_

### CO-265 — Extract universe-specific modules out of co-web/src/ — separate co (platform) from universes (extensions) (#81)
_Merged: 2026-05-22_
_Release: v2.25.0_

_(no acceptance criteria in spec)_

### CO-224 — Promote routes into context folders (auth/, content/, social/, admin/)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-263 — Feature-gate R2 deployer adapter to avoid AWS SDK bloat in default build (#79)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-262 — Fix CO-261 entries visibility — seed walker inserts rows but /entries API returns 0 (#78)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-260 — Cross-version changelog viewer — range queries + group-by-type + sort-by-PR-size
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-261 — Sync repo work/*.md → CO universe entries (live dev board for /co, /yggdrasil, /rfq, etc.) (#76)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-134 — Deployer adapter trait + first impl (static-on-R2)
_Merged: 2026-05-21_
_Release: v2.13.1_

- [ ] `DeployerAdapter` trait in `co-core::deploy`
- [ ] `StaticOnR2Adapter` with full deploy + rollback + status
- [ ] Unit tests with mock S3 client (via `mockall` or similar)
- [ ] Integration test: real R2 bucket, deploy a 3-file fixture universe, fetch the landing page, assert content
- [ ] `co deploy --target static-on-r2` CLI subcommand wired
- [ ] Rollback test: deploy v1, deploy v2, rollback to v1, verify v1 served

### CO-243 — VS Code (and LSP) integration — open universe as remote workspace (#75)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-259 — Split sidebar.js + state.js + api.js into smaller files for parallel task independence (#74)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-245 — Inline code editor for plaintext file types (CodeMirror) (#72)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-244 — Python / R REPL interoperability — DuckDB attach + in-browser REPL (#71)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-133 — deploy.yaml schema + universe-level manifest validation (#70)
_Merged: 2026-05-21_
_Release: v2.13.1_

- [ ] JSON Schema (or equivalent) for `deploy.yaml` v1 committed under `work/schema/deploy.v1.json`
- [ ] Rust struct `Manifest` derived (`serde` + `schemars`); round-trip-stable
- [ ] Validation called on universe save and on `co deploy` invocation; errors point to file + line + path
- [ ] 10 fixtures in `tests/fixtures/deploy/`: 5 valid, 5 invalid (each invalid one tests a specific error)
- [ ] `co validate deploy` CLI subcommand
- [ ] Docs: `docs/DEPLOY-MANIFEST.md` with one full example per target

### CO-242 — Unified file listing — surface all file types in universe entries (PDF, image, video, code) (#69)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-236 — co auth — CLI commands for centralized password reset + API token lifecycle (#68)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-129 — Jujutsu-shaped changelog renderer (op-log → commit DAG) (#67)
_Merged: 2026-05-21_
_Release: v2.13.1_

- [ ] DAG renderer component reads `/api/v1/universes/{u}/oplog` and produces an SVG/Canvas timeline
- [ ] Each node shows: op id (short), author, timestamp, change summary
- [ ] Click a node → side panel with full diff + "Restore to here" button
- [ ] Conflicts (CO-128 outputs) appear as distinct node types
- [ ] Branch labels rendered when present
- [ ] Performance: 1000-node DAG renders in <200ms (virtualized)
- [ ] Theme-aware (matches all 12 CO themes)

### CO-225 — Document AppState composition pattern + add a MODULES.md (#66)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-258 — co-auto agent prompts — forbid CHANGELOG.md + Cargo.toml mutations (release commits own them) (#65)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-257 — ship-task.sh + gh wrapper — never --delete-branch when mergeable is UNKNOWN (#64)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-253 — Unsubscribed universe = read-only + Subscribe/Login-to-Subscribe prompt on CRUD (#62)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-254 — Rename tutorial project + tasks to remove the CO/CO collision (template vs co dev) (#61)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-255 — Splitter resize — mirror behavior when detail pane is on the right (obsidian-mode) (#60)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-252 — Surface 'co' dev sub-universe on the anonymous sidebar + landing (#58)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-250 — Playwright static-asset MIME + bootstrap smoke (would have caught 2.13.3/4 cascade) (#57)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-251 — Template wiki rewrite — index.md as a hyperlinked landing + 'Ver mais' popular list (#56)
_Merged: 2026-05-21_
_Release: v2.13.1_

_(no acceptance criteria in spec)_

### CO-241 — Add true content-volume metrics (lines, words, chars) — fix 'lines = files' confusion (#55)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-239 — Fix host stats — wire nix::sys::statvfs for data_dir_total / data_dir_available (#54)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-238 — Sidebar UX — clarify owned/member/role/sub-universe semantics (#53)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-240 — Fix per-universe data_db_bytes — currently 0 for every universe (#51)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-232 — Cross-universe deep-link returns universe home instead of 404
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-237 — Hash API tokens at rest (currently stored in plain text) (#50)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-226 — Add OpenAPI coverage for auth + admin + chat to the interactions registry (#48)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-233 — Sync pipeline — latest changes not appearing on prod web (#47)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-221 — Slim AppStateInner (18 fields) via segregated sub-states (#45)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-219 — Promote chat_routes.rs (2063 LoC) into chat/ module folder (#44)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-223 — Define shared Worker trait + lifecycle (embedding + notification + push workers) (#43)
_Merged: 2026-05-20_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-222 — Unify auth gating into a single typed extractor (#42)
_Merged: 2026-05-19_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-220 — Replace cross-feature direct calls with in-process event bus (#40)
_Merged: 2026-05-19_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-217 — Top-20 serde_json::Value handler payloads → typed Deserialize structs (#39)
_Merged: 2026-05-19_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-249 — Gari ops universe — multi-platform cleanup with transparency + CO integration (#38)
_Merged: 2026-05-19_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-95 — Universe branching — materialized dev branches with deterministic copy + parallel deploys (#37)
_Merged: 2026-05-19_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-215 — Split server.rs into server/{router, state, validation, uat_boot, seed_orchestrator}.rs (#32)
_Merged: 2026-05-18_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-248 — co-auto --auto-pr — push branch + open PR after each successful task (#33)
_Merged: 2026-05-18_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-247 — Fizzy compatibility for CO quadro — import / export / shared schema (#31)
_Merged: 2026-05-18_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

### CO-234 — Chat shows 'Entre para participar' for logged-in users (HttpOnly cookie + JS detection mismatch) (#28)
_Merged: 2026-05-18_
_Release: v2.11.2_

_(no acceptance criteria in spec)_

## Carried Over

- (none tracked — retrospective simulation uses merge commits only)
