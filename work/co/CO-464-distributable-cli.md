---
title: Distributable `co` CLI — crates.io publish + quickstart + self-host onboarding
status: todo
---

# CO-464 — Distributable `co` CLI (publish + getting-started)

> Make the "download via cargo / GitHub" path real for new users. Today the
> hosted platform (sign up by email → web CRUD → optional CLI sync) works, but
> distribution of the CLI/self-host is half-wired.

## Done (this increment)
- **AGPL-3.0 `LICENSE`** added at repo root (matches the long-declared
  `license = "AGPL-3.0-or-later"` in Cargo.toml + `docs/licensa.md`).
- README install fixed: `cargo install --git https://github.com/artelonga/co co-cli`
  (the form that actually works today — `co-cli` builds installably with its
  path-deps when cargo clones the whole repo).

## Remaining
1. **crates.io publish of `co-cli`** is blocked: it path-depends on `co` (core),
   `co-engine`, `co-web`. To `cargo publish` the binary crate, those must be
   published too (or co-cli must be slimmed to not pull co-web). Decide: publish
   the crate graph, or keep `cargo install --git` as the supported install and
   drop the aspirational `cargo install co-cli` from docs.
2. **`install.sh`** referenced in README (`curl …/install.sh | sh`) + pre-built
   release binaries — verify they exist / wire the release workflow.
3. **Getting-started doc** — one page covering the three personas:
   hosted (email signup → web), CLI (`co auth`/`co sync` against a server),
   self-host (`co serve` / `co-local.sh`, folder=universe, code-to-stdout login).
4. **Self-host login UX** — without `RESEND_API_KEY` the magic-code prints to the
   server log; a `co serve --print-login` or first-run token would be friendlier.
5. Fold `scripts/co-local.sh` / `co-login.sh` into `co serve` proper so a stranger
   doesn't need repo scripts.
