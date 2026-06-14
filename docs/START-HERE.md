# 🚀 Start Here — CO in 5 minutes
### for the curious newcomer, tomorrow morning

> CO is a **graph-based content platform**: every universe is a folder of Markdown that
> can be served as a site, walked as a 2D world, and explored as an API. Pick a door.

---

## 🌐 Web — just open it
**`https://co.artelonga.com.br`** — no signup needed to look around.

- **The board** loads with a tutorial universe. Drag a task, make a task, switch among **12 themes**, toggle 🇧🇷/🇬🇧.
- **The Sala** (canvas) — a spatial view of your content; descend into universe-nodes (it's fractal).
- **`/api/docs`** — the live **Swagger UI**: every endpoint, try-it-in-browser.
- **`/shannon`** *(once deployed)* — the information-theory dashboard. Start here if you like being surprised.
- Curious about Yggdrasil? **`yggdrasil-artelonga.fly.dev/mundo`** — *walk* a universe with WASD.

**Anonymous → yours:** create up to 100 entries with no account; sign up (username + password, email optional) to claim your universe and go unlimited.

---

## 🔌 API — the contract is documented
Base: `https://co.artelonga.com.br/api/v1`

- **Discover it:** open **`/api/docs`** (Swagger) or fetch **`/api/openapi.json`** (OpenAPI 3.1).
- **Envelope (opt-in):** send header `X-API-Envelope: 1` to get `{ data, meta, errors }`; omit it for the raw shape. Every response carries `X-API-Version: 1.0`.
- **Auth:** `POST /api/v1/auth/signup` → `POST /api/v1/auth/password-login` → session cookie, or mint an API token (`POST /api/v1/auth/token`) with **least-privilege scopes** (e.g. `telemetry:read`, `entries:read`).
- **First calls:**
  ```bash
  curl https://co-artelonga.fly.dev/api/health
  curl https://co-artelonga.fly.dev/api/v1/universes/template
  curl -H 'X-API-Envelope: 1' https://co-artelonga.fly.dev/api/v1/auth/login-options
  ```
- **Source of truth:** `docs/architecture/api-catalog.md` → generates `openapi.yaml`. Never edit the YAML by hand.

---

## ⌨️ CLI — `co`
Install from the repo (`cargo install --path co-cli`), then:

```bash
co init <name>          # create a universe (a folder of Markdown)
co new task "Plan it"   # create content
co show <item>          # render it
co locate --type task   # search the graph
co validate all         # check the workspace
co serve                # run it locally as a site
co space list           # list your spaces
```

Full command map: `co --help`, and `co/CLAUDE.md` for the developer workflow.

---

## 🧭 Where to read next
| You want… | Go to |
|---|---|
| The big picture (use cases, content×form) | `docs/GUIA-DO-USUARIO.md` |
| Every HTTP route | `/api/docs` or `docs/architecture/api-catalog.md` |
| Server module conventions | `co-web/src/MODULES.md` |
| What just shipped | `docs/sprint/SPRINT-2026-06-14.md` |
| Operations / deploy / disk recovery | `docs/OPERATIONS.md` |

> **The CO philosophy:** your universe is *yours* — a plain folder you can read, walk,
> serve, and own. We're replacing the lock-in, not adding another one.
