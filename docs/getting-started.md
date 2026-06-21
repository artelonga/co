# Getting started with CO

CO meets you at three levels. Pick the one that fits.

## 1. Hosted (no install) — sign up and write in the browser

The fastest path. Nothing to install.

1. Go to **https://co.artelonga.com.br**.
2. Sign in with your e-mail — a one-time code is sent (no password needed).
3. On first login you automatically get your own **private personal universe**.
4. Create content in the web editor; it saves to your universe via the Vault API.

Public universes are discoverable and subscribable; your personal universe is
private by default.

## 2. CLI — drive a server from your terminal

Install the CLI (AGPL-3.0):

```bash
# one-line binary installer (macOS / Linux)
curl -fsSL https://co.artelonga.com.br/install.sh | sh

# …or build from source (any platform with a Rust toolchain)
cargo install --git https://github.com/artelonga/co co-cli
```

> There is **no `cargo install co-cli` from crates.io** — the CLI embeds the
> full server (`co serve`), so it is not an idiomatic crates.io library. The two
> commands above are the supported installs. (CO-464)

Authenticate against a server and sync a folder:

```bash
co login                      # e-mail magic-code against co.artelonga.com.br
co sync pull <universe>       # fetch a universe to a local folder
co sync push <universe>       # push local edits back
co sync watch <universe>      # live two-way sync
```

## 3. Self-host — run the whole platform locally

`co serve` embeds the server + SPA + SQLite. No Docker, no Fly, no account.

```bash
co serve --open               # start + open in your browser (127.0.0.1:54321)
co serve --port 8080          # custom port
co serve --data-dir ~/my-co   # custom data directory
```

Without a `RESEND_API_KEY`, login magic-codes are printed to the server log
(local dev). Drop any folder with markdown into your workspace and it becomes a
universe — no registry, no `co create` ceremony (see CO-466/CO-467).

---

**Versions & updates.** The one-line installer fetches the latest **tagged
GitHub release** binary; `cargo install --git` builds `main`. License: AGPL-3.0.
