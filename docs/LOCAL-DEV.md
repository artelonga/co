# Local Development

Run and test co-web on your own machine for development — loopback-only, dev
secrets, no deploy. (To actually *host* the platform for others, see
[`docs/SELF-HOST.md`](SELF-HOST.md); this doc is for hacking on the code.)

---

## Prerequisites

- **Rust toolchain** (`cargo`/`rustc`) — `rustup` or Homebrew.
- **Node + npm** — only for the web component tests / Playwright e2e.
- **`sqlite3`** CLI — handy for poking at `meta.db` / per-universe DBs.
- **(Optional) Ollama** at `:11434` for local agent/LLM testing.

---

## Quick start — run co-web on localhost

`CO_ENV=local` makes co-web bind `127.0.0.1` (loopback only). The dev `JWT_SECRET`
fallback is allowed in `local` (CO-469 only panics in prod):

```bash
cd ~/projects/co

CO_ENV=local \
JWT_SECRET=dev-local-secret \
CO_WEB_DATA=~/.co/local-data \
CO_WEB_PORT=3000 \
  cargo run -p co-web

# Health check
curl -s http://localhost:3000/api/health
```

Data uses the CO-77 layout under `$CO_WEB_DATA`: `meta.db` + `universes/<key>/data.db`.

### Convenience dev scripts

| Script | What it does |
|--------|--------------|
| `scripts/co-local.sh` | Serve your whole `~/projects` workspace — every top-level folder with a `_universe.yaml` auto-registers as a universe at `/co/<key>`. Release build, port 3000, owner login as yuri. |
| `scripts/pr-localhost.sh <pr\|branch>` | **Isolated per-PR** deploy: own git worktree, deterministic port (band 8900-9699), own data dir — run several PRs side by side without clobber. `--build-only`, `--port N`, `--stop`, `--list`, `--with-bot` (also boots the WhatsApp bot bridge pointed at this co-web). |
| `scripts/ab-localhost.sh <pr\|branch>` | A/B-live launcher: brings up CO (A) via `pr-localhost.sh` and yggdrasil (B) side by side over the same shared substrate; `--stop` tears both down. |

```bash
# Serve the local workspace
scripts/co-local.sh

# Review a PR in a real browser, isolated
scripts/pr-localhost.sh 512                 # build + run, returns when healthy
scripts/pr-localhost.sh 512 --with-bot      # + WhatsApp bot bridge
scripts/pr-localhost.sh --list              # active per-PR deployments + ports
scripts/pr-localhost.sh 512 --stop          # kill + remove worktree + temp data

# Compare CO and yggdrasil live on the same branch
scripts/ab-localhost.sh my-branch
```

---

## Running tests

```bash
# Rust unit + integration tests
cargo test
# or via the layered runner:
scripts/co-test lib

# Formatting + lints (must be clean before committing)
cargo fmt
cargo clippy -- -D warnings
```

### Web component tests (Vitest, no server, no browser)

```bash
cd co-web
npm ci
npm run test:components        # or: scripts/co-test components
```

### Playwright e2e (real browser + real server)

```bash
cd co-web
npm ci                         # install web deps
npx playwright install         # one-time: download browsers
cargo build -p co-web          # the e2e harness runs the built binary

# Run (auto-starts co-web on its own port; baseURL defaults to localhost:3000,
# override with BASE_URL):
scripts/co-test smoke          # quick health + page-load smoke (< 1 min)
scripts/co-test e2e            # full per-feature suite
scripts/co-test e2e --since main   # only specs changed since main
```

The e2e layer overview (which test goes where) is in
[`co-web/e2e/README.md`](../co-web/e2e/README.md).

### Smoke scripts

```bash
scripts/smoke-lib.sh           # library-level smoke
scripts/co-test smoke          # Playwright smoke against a local server
# (scripts/smoke-prod.sh targets production — not for local dev)
```

---

## Local agent / LLM testing (Ollama)

co-web talks to an OpenAI-compatible endpoint via `CO_OLLAMA_URL`
(default `http://localhost:11434`). Run a model locally and point co-web at it:

```bash
# Start Ollama + a model (see ~/projects/tools/local-inference for the guarded runner)
ollama pull qwen3-coder:30b
ollama run qwen3-coder:30b        # warms the model

# co-web will use http://localhost:11434 by default; override if needed:
CO_ENV=local JWT_SECRET=dev-local-secret CO_OLLAMA_URL=http://localhost:11434 \
  cargo run -p co-web
```

> 36 GB is tight — prefer a 14B model when coding alongside it; use the memory-
> guarded `~/projects/tools/local-inference/run-test.sh`. Free memory anytime with
> `ollama stop <model>`.

---

## See also

- Self-hosting (real deploy): [`docs/SELF-HOST.md`](SELF-HOST.md).
- Project conventions, TDD, versioning: [`CLAUDE.md`](../CLAUDE.md).
