# CO

**CO** is an open-source, graph-based content management platform. You write Markdown files with YAML frontmatter; CO indexes them into a graph, serves a kanban board and wiki, and syncs edits in real time. Run it locally with the CLI, self-host the web server, or use the hosted version at [artelonga.com.br/co](https://artelonga.com.br/co).

---

<!-- Screenshot: template board at artelonga.com.br/co -->
> **Hosted demo:** [artelonga.com.br/co](https://artelonga.com.br/co) — create a free universe, no login required (up to 100 entries).

---

## Documentação

| Documento | Quando ler |
|-----------|-----------|
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Entender componentes, fluxo de dados, modelo de acesso e temas |
| [docs/OPERATIONS.md](docs/OPERATIONS.md) | Deploy, logs, backup, rotação de segredos e recovery |
| [docs/ONBOARDING.md](docs/ONBOARDING.md) | Configurar e rodar o CO localmente a partir do código-fonte |

---

## Run locally

The fastest way to run CO on your own machine — no Docker, no Fly account, no Rust toolchain required:

```bash
# macOS / Linux — one-line install
curl -fsSL https://co.artelonga.com.br/install.sh | sh

# Start the server and open in your default browser
co serve --open
```

CO binds to `127.0.0.1:54321` by default. Your data lives in the platform data dir (`~/.local/share/co` on Linux, `~/Library/Application Support/co` on macOS). Everything runs offline against a local SQLite database.

```bash
co serve --port 8080              # custom port
co serve --data-dir ~/my-co       # custom data directory
co serve --public                 # expose on local network (prints a warning)
```

Pre-built binaries are available for macOS (Intel + Apple Silicon), Linux (x86_64 + ARM64), and Windows (x86_64) on the [releases page](https://github.com/artelonga/co/releases).

---

## Quick Start

### Option A — CLI (Rust)

```bash
cargo install co-cli
co init meu-projeto
cd meu-projeto
co new task "primeira tarefa"
co show board
```

### Option B — Docker

```bash
docker run -d \
  -p 3000:3000 \
  -v co-data:/data \
  -e JWT_SECRET=change-me \
  ghcr.io/artelonga/co:latest
```

Open [http://localhost:3000/co](http://localhost:3000/co).

---

## Self-Hosting

### Docker Compose (recommended)

```yaml
services:
  co:
    image: ghcr.io/artelonga/co:latest
    ports:
      - "3000:3000"
    volumes:
      - co-data:/data
    environment:
      JWT_SECRET: ${JWT_SECRET}
      CO_WEB_DATA: /data
      CO_WEB_PORT: 3000
    restart: unless-stopped

volumes:
  co-data:
```

```bash
JWT_SECRET=$(openssl rand -hex 32) docker compose up -d
```

### Fly.io

```bash
git clone https://github.com/artelonga/co
cd co
fly launch --no-deploy
fly secrets set JWT_SECRET=$(openssl rand -hex 32)
fly deploy
```

### Build from source

```bash
git clone https://github.com/artelonga/co
cd co
cargo build --release -p co-web
./target/release/co-web
```

---

## Architecture

```
┌─────────────┐     REST / WebSocket     ┌─────────────────┐
│   co-cli    │ ◄──────────────────────► │    co-web       │
│  (Rust CLI) │                          │  (Axum server)  │
└──────┬──────┘                          └────────┬────────┘
       │                                          │
       └──────────────────┬───────────────────────┘
                          │
                   ┌──────▼──────┐
                   │    core     │
                   │ (Rust lib)  │
                   │             │
                   │  graph DB   │
                   │  Markdown   │
                   │  SQLite     │
                   └─────────────┘
```

| Component | Description |
|-----------|-------------|
| `core`    | Graph database, Markdown parser, content types, validation |
| `co-cli`  | Command-line interface — init, new, show, validate |
| `co-web`  | Axum HTTP server — board UI, REST API, WebSocket CRDT sync |

---

## CLI Reference

```bash
co init <name>          # Create a new universe
co new task "title"     # Create a task
co new note "title"     # Create a note
co show board           # Open board in browser
co locate               # Search content
co locate --type task   # Filter by type
co validate all         # Validate workspace
co schema list          # List content types
co config show          # Show configuration
```

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, branch conventions, and the PR process.

### Merging PRs safely

Use `scripts/safe-merge-pr.sh` instead of `gh pr merge --delete-branch` directly.
GitHub sometimes returns `mergeable: UNKNOWN` (transient) — bare `gh pr merge --delete-branch`
will fail the merge but still delete the branch, silently closing the PR without merging.
The wrapper polls every 5s (up to 30s) until the status is determinable:

```bash
scripts/safe-merge-pr.sh artelonga/co <pr-number>
```

All contributions are welcome — bug reports, feature requests, documentation, and code.

---

## License

[GNU AGPL v3 or later](LICENSE) — Copyright (c) 2025–2026 Institutional PointSet / ArteLonga.

The AGPL network clause applies: if you run a modified version of CO as a
network service, you must offer its source to users of that service.
See [`/co/template?page=licensa`](https://co.artelonga.com.br/co/template?page=licensa)
for a plain-language explanation.

The Obsidian plugin (`co-obsidian/`) remains MIT — it's a client tool, not
a network service, and Obsidian's plugin ecosystem expects permissive
licenses.
