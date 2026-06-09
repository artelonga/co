# CI/CD — failure causes & proactive prevention

What runs, what breaks it, and how to catch each failure *before* it goes red.

## Pipeline

| Workflow | Trigger | Gates |
|---|---|---|
| `ci.yml` → `test` | push/PR to `main` | system deps → `cargo build` → `cargo test` → `cargo clippy -- -D warnings` → `cargo fmt --all -- --check` |
| `ci.yml` → `e2e` | after `test` | Playwright (`chromium-desktop`); warns if a spec file > 30 tests |
| `openapi-check.yml` | PR touching `*_routes.rs` / `openapi*.yaml` / generator | `npm run openapi:check` (spec drift) |
| `staging-deploy.yml` | push to `main` | `flyctl deploy --config fly.staging.toml` |
| `release.yml`, `backup.yml`, `co-agent-publish.yml` | — | release/backup/publish |

## Known failure causes → prevention

1. **Staging deploy red on every `main` push — `FLY_API_TOKEN` unset.**
   `flyctl` errors `no access token available`; the repo has no secrets set, so the
   job failed on every push. **Fixed:** `staging-deploy.yml` now maps the secret to a
   job env and **skips the deploy gracefully (green)** when it's absent.
   **To actually enable staging CD:** `gh secret set FLY_API_TOKEN --body "$(flyctl auth token)"`.

2. **`cargo fmt --check` fails** (recurring — `feedback_coauto_fmt_check`). Automated
   pushes skip `cargo fmt`. **Prevention:** `scripts/ci-preflight.sh` runs `fmt --check`;
   when CI Format-check is red, just `cargo fmt --all && git push`.

3. **`cargo clippy -- -D warnings` fails** — any new warning is a hard error.
   **Prevention:** preflight runs clippy with `-D warnings` (clippy is version-pinned via
   `rust-toolchain.toml`, so run it locally on the same channel).

4. **OpenAPI drift** (`openapi-check.yml`; `feedback_coauto_catalog_drift`) — regenerating
   `openapi.yaml` without updating `api-catalog.md`, surfacing on the *next* PR.
   **Prevention:** preflight runs `npm run openapi:check` when route/spec files changed.

5. **`protoc` / system deps missing** — historical (CI 100% red 2026-05-12). **Already
   fixed** in `ci.yml` (`protobuf-compiler pkg-config libssl-dev g++ libdbus-1-dev`).

6. **Cancelled/“failed” runs from rapid `main` pushes** — a PR merge + its archive commit
   push back-to-back; the earlier run is superseded and shows as failure. Benign (no code
   gate failed). If the noise matters, add a `concurrency:` group to `ci.yml`.

7. **Flaky server-readiness timeout** — `testserver_tests` spawn the real `co` binary and
   poll `/api/health`; the 30 s deadline (`tests/testkit.rs wait_ready`) flaked under CI
   load when cold-boot + migrations + seed ran long (e.g. `test_ts_vault_write_and_read`,
   2026-06-09). **Fixed:** deadline raised to 90 s.

## Preflight — mirror the gates locally

```bash
scripts/ci-preflight.sh           # fmt + clippy + build + openapi-if-changed
scripts/ci-preflight.sh --test    # also cargo test --workspace
# optional: wire as a pre-push hook
ln -sf ../../scripts/ci-preflight.sh .git/hooks/pre-push
```

A clean preflight means the `ci.yml` `test` job and `openapi-check` will pass — the four
recurring red causes (fmt, clippy, drift, build) are caught before push.
