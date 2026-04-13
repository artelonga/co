# CO

**CO** is an open-source, graph-based content management platform. You write Markdown files with YAML frontmatter; CO indexes them into a graph, serves a kanban board and wiki, and syncs edits in real time. Run it locally with the CLI, self-host the web server, or use the hosted version at [artelonga.com.br/co](https://artelonga.com.br/co).

---

<!-- Screenshot: template board at artelonga.com.br/co -->
> **Hosted demo:** [artelonga.com.br/co](https://artelonga.com.br/co) — create a free universe, no login required (up to 100 entries).

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

All contributions are welcome — bug reports, feature requests, documentation, and code.

---

## License

[MIT](LICENSE) — Copyright (c) 2025 Institutional PointSet
