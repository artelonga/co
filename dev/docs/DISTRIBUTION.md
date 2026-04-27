# CO Distribution: Open Source vs Proprietary

## Architecture Boundary

```
┌─────────────────────────────────────────────────────────┐
│  OPEN SOURCE (MIT)  ·  github.com/institutional-pointset/co  │
│                                                         │
│  core/          Graph engine, types, validation         │
│  co-cli/        CLI tool (co init, co new, co validate) │
│  co/            Automation engine, agents, tools        │
│  game-core/     Game framework, plugins                 │
│  co-web/        Web server (board, auth, universo API)  │
│    ├── auth.rs           JWT + email verification       │
│    ├── universo.rs       Universo trait (generic)        │
│    ├── iceberg.rs        Manifest generation (generic)   │
│    ├── github_auth.rs    GitHub PAT middleware (generic)  │
│    ├── gestao_routes.rs  Content CRUD via GitHub (generic)│
│    ├── models.rs         Board: projects, tasks          │
│    ├── storage.rs        SQLite migrations (generic)     │
│    ├── experiment.rs     A/B testing framework           │
│    ├── game_*.rs         Game API routes + models        │
│    └── server.rs         Route mounting, middleware       │
│                                                         │
│  work/          Work item schemas (generic)             │
│  agents/        Agent definitions (generic)             │
│  tools/         Tool definitions (generic)              │
│  openapi.yaml   Full API specification                  │
│                                                         │
├─────────────────────────────────────────────────────────┤
│  PROPRIETARY  ·  Private repos / not distributed        │
│                                                         │
│  co-web/                                                │
│    ├── quilombo_models.rs      Community data types      │
│    ├── quilombo_routes.rs      Community API endpoints    │
│    ├── quilombo_storage.rs     Community SQLite schema    │
│    └── quilombo_permissoes.rs  Community role permissions │
│                                                         │
│  quilombo/          Instance content (in co repo)       │
│    ├── schema.yaml          Content type definitions     │
│    ├── .universo.yaml       Universe identity            │
│    ├── relatos/             Community stories            │
│    └── paginas/             Static pages                 │
│                                                         │
│  quilomboaraucaria/   Universe repo (separate)          │
│    ├── .co/                 Iceberg metadata             │
│    ├── relatos/             Published stories            │
│    ├── eventos/             Community events             │
│    ├── membros/             Member profiles              │
│    ├── quadro/              Missions with status         │
│    ├── jardim/              Knowledge garden             │
│    └── modelos/             Content templates            │
│                                                         │
│  quilombo-blog/       SvelteKit frontend (separate)     │
│    ├── src/routes/          Page layouts & UX            │
│    ├── src/lib/components/  UI components                │
│    ├── src/app.css          Design system & palette      │
│    └── static/              Assets, fonts, images        │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

## What ships as open source

The `co` repo on GitHub contains the **platform** — everything needed to run
your own content universe. Anyone can:

- `co init meu-projeto` — create a content workspace
- `co board` — launch the web board with task management
- Use the Universo trait to serve any markdown directory
- Use the gestao API to CRUD content via GitHub auth
- Use the Iceberg metadata layer for schema evolution
- Build games with the plugin system
- Run A/B experiments

### Open source API endpoints

| Prefix | Purpose |
|--------|---------|
| `/api/health` | Server status |
| `/api/projects/**` | Board: projects, tasks, comments, activity |
| `/api/experiment/**` | A/B variant assignment and feedback |
| `/api/v1/auth/**` | Email code + JWT authentication |
| `/api/v1/games/**` | Game leaderboards, stats, profiles |
| `/api/v1/gestao/**` | Content CRUD via GitHub PAT (generic) |

### Open source crates

| Crate | License | Purpose |
|-------|---------|---------|
| `co` (core) | MIT | Graph engine, types, validation, features |
| `co-cli` | MIT | CLI tool |
| `co` (automation) | MIT | Agent engine, tools, context |
| `co-web` | MIT | Axum web server (generic modules) |
| `game-core` | MIT | Game framework with plugin system |

## What is proprietary

Universe **instances** and **frontends** are private. They contain:

- **Content** — the actual markdown files (relatos, eventos, membros)
- **Layout** — SvelteKit components, CSS design system, page templates
- **Community features** — the `quilombo_*` modules that implement
  mission participation, member roles, messaging, telemetry
- **Identity** — `.universo.yaml`, color palette, typography, branding

### Proprietary API endpoints

| Prefix | Purpose | Repo |
|--------|---------|------|
| `/api/v1/quilombo/**` | Community features (auth, members, missions, comments, messages) | co (quilombo_* modules) |

### Proprietary repos

| Repo | Visibility | Contains |
|------|-----------|----------|
| `artelonga/quilomboaraucaria` | Private | Universe content + Iceberg metadata |
| `artelonga/quilombo-blog` | Private | SvelteKit frontend, design, layout |
| `artelonga/ArteLonga` | Public | Content universe (markdown only) |

## Separation rule

**If it works for any universe → open source.**
**If it's specific to Quilombo Araucária → proprietary.**

| Question | Answer | License |
|----------|--------|---------|
| Can another community use this code as-is? | Yes → MIT |
| Does it reference quilombo, araucária, or specific people? | Yes → Proprietary |
| Is it a generic pattern (CRUD content, auth, board)? | Yes → MIT |
| Is it a specific design, layout, or color palette? | Yes → Proprietary |
| Is it markdown content written by community members? | Yes → Proprietary |

## Extracting proprietary code

The `quilombo_*` modules in `co-web/src/` should eventually move to a
separate crate or feature flag:

```toml
# co-web/Cargo.toml (future)
[features]
default = []
quilombo = []  # enables quilombo_* community modules
```

For now they coexist in co-web but are clearly namespaced with the
`quilombo_` prefix and mounted behind `/api/v1/quilombo/`.
