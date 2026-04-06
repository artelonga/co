# Changelog

All notable changes to CO are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.28.0] — 2026-04-06

### co (workspace)

#### Added — CO-28: Open source repo setup

- `README.md` — rewritten for public audience: what CO is, quick start (cargo install + Docker), self-hosting (Docker Compose + Fly.io), architecture diagram, CLI reference, contributing link
- `CONTRIBUTING.md` — development setup, TDD workflow, branch/label conventions, commit format, test rules, PR process
- `.github/ISSUE_TEMPLATE/bug_report.md` — structured bug report template
- `.github/ISSUE_TEMPLATE/feature_request.md` — feature request template with acceptance criteria
- `.gitignore` — added `*.db`, `*.redb`, `.env`, `.env.local` patterns; removed `!co-web/data/` exception that could allow committing runtime databases
- `Cargo.toml` — added `keywords` and `categories` to workspace package; updated repository URL to `artelonga/co`

---

## [0.27.0] — 2026-04-06

### co-web

#### Added — CO-33: E2E test suite — Playwright for full MVP flow

- `e2e/universe.spec.ts` — Universe creation: criar form submit → redirect to /co/:slug → editable board
- `e2e/board-drag.spec.ts` — Board drag-and-drop between kanban columns + full CRUD sequence
- `e2e/codemirror.spec.ts` — CodeMirror 6 editor: init, toolbar (Bold/Italic/Heading), live preview, save+persist
- `e2e/usage-gate.spec.ts` — Usage gate: API 402 structure, overlay DOM, "Entrar" opens login modal
- `e2e/theme.spec.ts` — Palette switcher: anonymous sees 4, switch updates CSS vars without reload
- `e2e/i18n.spec.ts` — i18n toggle pt↔en, co_lang cookie set, persists across page reload
- `e2e/auth-crdt.spec.ts` — Auth flow, sharing gate, anonymous editor has no WebSocket, CRDT two-context sync
- `e2e/responsive.spec.ts` — Board renders at mobile (375px), tablet (768px), desktop (1280px) viewports
- `.github/workflows/ci.yml` — Added `e2e` job: build co-web → install Playwright → run Chromium suite → upload HTML report

---

## [0.26.0] — 2026-04-06

### co-deploy

#### Added — CO-32: Ansible deployment — provision, deploy, backup playbooks for Fly.io + VPS

- New `co-deploy/` directory with standard Ansible structure
- `inventory/fly.yml` — Fly.io target (local connection via flyctl)
- `inventory/vps.yml` — generic VPS target (DigitalOcean, Hetzner, etc.) with env-var overrides
- `playbooks/provision.yml` — creates `co` unprivileged user, installs ca-certificates + sqlite3 + zstd + Caddy, creates `/opt/co/` + `/var/lib/co/data/`, configures UFW (allow 80/443, deny rest)
- `playbooks/deploy.yml` — cross-compiles co-web via `cross`, copies binary, writes systemd unit, runs seed SQL on first deploy, restarts service, verifies `/api/health`
- `playbooks/backup.yml` — SQLite `.backup` (online, consistent), zstd compression, 7 daily + 4 weekly rotation, optional rclone upload to S3/B2, cron at 03:00 UTC
- `playbooks/fly-deploy.yml` — wraps `flyctl deploy --remote-only` with pre-deploy snapshot and post-deploy health check
- `templates/co-web.service.j2` — systemd unit with ExecStart, WorkingDirectory, Environment, systemd hardening (NoNewPrivileges, ProtectSystem)
- `templates/caddy.conf.j2` — reverse proxy with auto-SSL, zstd+gzip compression, security headers (HSTS, X-Frame-Options, etc.), static asset caching
- `group_vars/all.yml` — shared config: co_version, co_port, co_domain, backup retention settings
- `group_vars/production.yml` — ansible-vault encrypted secrets: JWT_SECRET, RESEND_API_KEY
- `molecule/default/` — Docker-based integration test (provision + stub deploy on Debian 12, idempotency check)
- `requirements.yml` — community.general + ansible.posix collections
- `README.md` — quickstart for VPS and Fly.io

---

## [0.25.0] — 2026-04-06

### co-web

#### Added — CO-31: CRDT sync — Yjs + WebSocket, login required

- New module `co-web/src/ws.rs`: `DocRoom` struct (yrs `Doc`, broadcast tx, client count, dirty notify), `DocRoomManager = Arc<RwLock<HashMap>>`, `ws_handler`, `handle_socket`
- `GET /ws/doc/:universe_slug/:doc_id` — JWT-gated endpoint; returns 401 for anonymous requests (token via `?token=` query param or `co_auth` cookie)
- Yjs sync protocol v1 (binary lib0 encoding): MSG_SYNC (0) with SYNC_STEP1/STEP2/UPDATE; MSG_AWARENESS (1) for cursor positions
- Room lifecycle: load content from SQLite on first connect (initializes Y.Doc), broadcast updates to all connected clients, debounced persist (5s idle), cleanup on last disconnect
- Heartbeat: ping every 30s, disconnect after 60s silence; rate limit: 100 messages/sec per client (token bucket)
- `AppStateInner.doc_rooms` field added; WS route mounted at `/ws/doc/{slug}/{doc_id}`
- `Storage::get_entry_body()` and `Storage::update_entry_body()` methods for CRDT persistence
- Sharing gate in `get_universe_info`: anonymous universes return 404 for non-owners (checked via `co_universe_owner` cookie)
- Frontend: added `yjs`, `y-codemirror.next`, `lib0` to editor bundle
- `createAwareness()` shim implementing y-codemirror.next's awareness interface (no y-protocols dep)
- `CoYjsProvider` class: WebSocket provider with reconnect, sync-step-1 on open, apply sync-step-2/update, forward awareness
- `initEditor` accepts `wsUrl` and `user` params; CRDT mode for logged-in users; anonymous mode shows "Crie uma conta pra colaborar" toast
- Collab badge ("N users editing"), connection status dot (green/yellow/red), remote cursor CSS
- 7 unit tests: varuint roundtrip, varbytes roundtrip, sync frame structure, rate limiter burst/block, DocRoom init, anonymous 401, two-user sync

---

## [0.24.0] — 2026-04-06

### co-web

#### Added — CO-30: Dynamic CSS engine — token generation from universe config at runtime
- New module `co-web/src/theme_engine.rs`: `ThemePreset` struct (name, tokens HashMap, font fields) + `generate_css()` function
- Five built-in presets with all required CSS tokens: `scholarly` (warm cream/bronze), `scholarly-dark` (dark chocolate/bronze), `relic` (near-black/rose), `relic-light` (off-white/burgundy), `modern` (default indigo)
- All presets define: `--bg`, `--sidebar-bg`, `--card-bg`, `--text-primary`, `--text-secondary`, `--accent`, `--border`, `--status-*`, `--priority-*`, `--font`, `--font-mono`, `--radius-*`, `--shadow-*`
- `generate_css(preset, overrides)` merges custom token overrides on top of preset, outputs deterministic `:root { … }` block
- `GET /api/v1/universes/:slug/theme.css` — returns generated CSS, `Cache-Control: no-cache`, ETag based on config hash, supports `If-None-Match` (304)
- Dark/light companion mapping: `scholarly` ↔ `scholarly-dark`, `relic-light` ↔ `relic`
- Frontend (variant a): `loadThemeCss(slug)` hot-swaps `<link id="co-theme-css">` href — no page reload when theme changes
- Frontend: custom fonts inject `<link rel="stylesheet" href="https://fonts.googleapis.com/…">` with preconnect hints
- Settings panel (owner only): added dark/light toggle button, `modern` theme option, custom token overrides JSON textarea
- Unit tests: 13 theme engine tests + 4 HTTP endpoint integration tests (200 OK, all tokens present, CSS changes on theme change, 404 for missing universe, ETag 304)

---

## [0.23.0] — 2026-04-06

### co-web

#### Added — CO-23: Usage gate — 100 entries free, then account required
- `universes.content_count` column (migration v11): cached counter incremented/decremented on writes and deletes
- Middleware-style `check_usage_gate` helper: returns 402 Payment Required for anonymous universes at or above 100 entries
- Anonymous write access: `clone_universe` issues an anon JWT session cookie + `co_universe_owner` cookie for claiming
- `POST /api/v1/universes/:slug/claim` — authenticated user claims an anonymous universe (cookie must match)
- `GET /api/v1/universes/:slug` — public universe info: `content_count`, `is_anonymous`, `is_template`
- 402 response body: `{ "error": "usage_limit", "message": "Crie uma conta para continuar", "message_en": "...", "current": N, "limit": 100 }`
- Frontend (variant a): 402 → usage limit modal with "Criar conta" / "Entrar" buttons; content count badge in header
- After login with anonymous universe: auto-claim transfers ownership to real user
- Unit test: 99 entries OK, 100th OK, 101st blocked (402), unblocked after claim

---

## [Unreleased] — co-web E2E Testing (UX-50 Epic)

### co-web

#### Added — UX-51: Initialize Playwright project
- Playwright + @axe-core/playwright devDependencies in `co-web/package.json`
- `playwright.config.ts` — baseURL localhost:3000, 9 projects (chromium/firefox/webkit × desktop/tablet/mobile)
- Custom viewports: desktop (1280×720), tablet (768×1024), mobile (375×812)
- `e2e/global-setup.ts` — builds binary, starts co-web, polls `/api/health`
- `e2e/global-teardown.ts` — SIGTERM cleanup, skips if external server
- `.gitignore` updated for node_modules, test-results, playwright-report
- `npx playwright test --pass-with-no-tests` exits cleanly (code 0)

---

## [0.22.1] - 2026-01-04

### Fixed
- **External Folder Support** (#77)
  - Bundle language configs in binary using `include_str!()`
  - CO now works properly in any registered workspace without source files
  - `co init` simplified to just create directory (no README.md)
  - `co new` defaults to current directory instead of 'en' space
  - Namespaces are now simple directories users organize however they want

## [0.22.0] - 2026-01-04

### Added
- **System-wide Installation & Namespace Detection** (#75)
  - `.co/` directory now recognized as CO workspace root marker
  - `co repo switch <alias>` to switch active workspace context
  - Git submodule detection for nested repositories
  - `is_submodule` field in `SpaceLocation::InSpace` variant
  - `is_git_submodule()` and `is_submodule()` helper methods
  - Enhanced `co space current` with helpful guidance when not in workspace
  - `effective_space()` method combining detected and active workspaces
  - `active_repo` field in `GlobalConfig` for workspace context persistence

### Changed
- `co space current` now shows "(switched)" indicator when using active workspace
- Status command shows "(submodule)" indicator when in a git submodule
- Improved error messages with actionable suggestions (Navigate, Register, Switch)

## [0.21.2] - 2026-01-04

### Changed
- **Rename ui/ to i18n/** (#72)
  - Renamed `ui/` folder to `i18n/` for clarity
  - Updated all path references in core and CLI
  - Folder now clearly indicates internationalization purpose

## [0.21.1] - 2026-01-04

### Added
- **Explicit Forbidden Character List** (#70)
  - `FORBIDDEN_ID_CHARS` constant documenting all forbidden ID characters
  - `is_valid_id_char()` function for character validation
  - `validate_id()` function to check ID strings for invalid characters
  - User-facing error messages in `co create` showing forbidden characters
  - Comprehensive tests validating all forbidden characters are handled

### Documentation
- Added doc comments explaining forbidden character categories:
  - Filesystem-unsafe: `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|`
  - Shell/special: `'`, `!`, `@`, `#`, `$`, `%`, `^`, `&`
  - Whitespace: space, tab, newline, carriage return
- Clarified allowed characters: alphanumeric, hyphen, dot, underscore

## [0.21.0] - 2026-01-04

### Added
- **Documentation System** (#42)
  - `co help` - Topic-based embedded documentation
  - `co help getting-started` - Quick start guide
  - `co help spaces` - Understanding spaces
  - `co help workflows` - Plan & Execute, Write workflows
  - `co help work-items` - User-stories, tasks, epics
  - Alias: `co h` for quick access
  - Added `clap_mangen` for future man page generation

### Changed
- Updated CLAUDE.md with work item types and git label mapping
- Clarified work item hierarchy (epic → user-story → task)
- Removed deprecated "scope" terminology from documentation

### Fixed
- Removed personal name references, using PRIVATE/PUBLIC/USER namespaces

## [0.20.0] - 2026-01-04

### Added
- **Archive & Storage** (#43)
  - `co archive <item>` - Move content to archive with deindexing
  - `co archive restore <item>` - Restore content from archive
  - `co archive list` - List all archived items
  - Directory structure mirrors original: `work/tasks/` → `work/archive/tasks/`
  - Adds `archived_at` timestamp to frontmatter
  - Adds `indexed: false` to exclude from co operations (locate, validate)
  - `--force` flag to replace existing archived items
  - Alias: `co ar` for quick access

## [0.19.0] - 2026-01-04

### Added
- **Analyze Command** (#41)
  - `co analyze <item>` - Evaluate content quality and generate suggestions
  - Checks for clear title, status field, and required sections
  - Type-aware validation: user-story (As/I Need/To), task (Given/When/Then)
  - Detects broken internal [[links]]
  - Generates actionable improvement suggestions
  - Generates interview questions for missing information
  - Colored output with ✓/⚠/✗ indicators
  - `--verbose` flag for detailed analysis

## [0.18.0] - 2026-01-04

### Added
- **Tools & Extensions** (#40)
  - `co tools run <name> [args...]` - Execute a tool with arguments
  - Tool types: `deterministic` (shell commands) and `predictive` (ML models, stub)
  - User tools in `user/tools/` take precedence over system tools
  - Tool schema extended with `tool_type` field
  - Default behavior: deterministic when `tool_type` not specified
  - Error handling: tool not found, missing command, execution failure

## [0.17.0] - 2026-01-04

### Added
- **Writer Agent System** (#39)
  - `co write <type> --agent <name>` - Generate content using writer agents
  - Agent backends: `manual` (interactive prompts), `claude` (skeleton for LLM), `ollama` (stub)
  - `--context FILE` to provide additional context from a file
  - `--in SPACE` to specify target space
  - `--name NAME` to skip name prompt
  - Agent schema extended with `backend` and `context` fields
  - New `agents/writer.md` example agent
  - Output validated against content schemas

## [0.16.0] - 2026-01-04

### Added
- **Plan & Execute Workflow** (#38)
  - `co conduct plan <objective>` - Create structured use-case proposals with acceptance criteria
  - `co conduct execute <id>` - Drive plans through git workflow states (todo → in-progress → review → done)
  - Two modes: Manual (interactive prompts) or Assisted (skeleton for LLM)
  - `--context FILE` to load context from a file
  - `--repo <alias>` for cross-repo operations
  - Auto-creates GitHub issue on plan creation
  - Branch creation on execute, PR tracking via `gh` CLI
  - Space-aware architecture with global repo registry

## [0.15.0] - 2026-01-04

### Added
- **GitHub as Source of Truth** (#36)
  - `co gh issue list` - List issues from GitHub repository
  - `co gh issue show <number>` - Show issue details
  - `co collab pull --all` - Pull all open issues to local markdown files
  - `co collab pull <number>...` - Pull specific issues
  - GitHub → CO mapping: labels to type/priority, assignees, state to status
  - New `core/src/github/` module with types, mapping, and GhCli wrapper

## [0.14.0] - 2026-01-04

### Added
- **Space Isolation & Commit Guards** (#47)
  - `SpaceLocation` detection: automatically detect if you're in a space or at repo root
  - `co status` now shows current location context (space vs repo root)
  - `co init --check` to find unprotected spaces (not gitignored)
  - Walking directory tree to find space markers (README.md with `type: space`)

### Changed
- Status command now displays location context with commit guard warnings

## [0.13.1] - 2026-01-04

### Changed
- **Terminology Refactor** (#49)
  - Standardized terminology: "Space" is the canonical term for namespace directories
  - Deprecated "scope" from system references (backwards-compatible aliases remain)
  - "Context" now exclusively refers to user-provided content/prompts
  - Renamed `core/src/scope.rs` → `core/src/space.rs`
  - Updated all CLI help text, commands, and i18n labels
  - Updated `type: context` → `type: space` in frontmatter
  - All tests and validation messages updated

## [0.13.0] - 2026-01-03

### Added
- **Collaborative Content Creation** (#48)
  - `co create` - Interactive content creation with role selection
  - User role: Structured prompts for user-stories (AS A / I NEED / SO THAT) and tasks (GIVEN / WHEN / THEN)
  - Agent role: Creates skeleton templates for Claude Code to fill in
  - `--story` flag to link tasks to parent user stories
  - `## Prompt` section for context persistence

## [0.12.2] - 2026-01-04

### Added
- CLAUDE.md development instructions (#56, #57)

### Changed
- Streamlined versioning workflow: version bump in same PR (#59)
- Added branch cleanup instructions

## [0.12.1] - 2026-01-04

### Added
- CHANGELOG.md with complete version history (#52)

### Changed
- Versioning policy: issues drive releases (#53)

## [0.12.0] - 2026-01-03

### Added
- **Spaces & Multi-Repo SSH** (#37, #45)
  - `co space list` - List all registered spaces
  - `co space current` - Show current space details
  - `co repo add --ssh-host` - Configure SSH identity per repo
  - Auto-detect current space from working directory
- **Extensible Content Types** (#35, #44)
  - Custom content types via `schema.yaml`
  - `co schema list` - List all available types (built-in + custom)
  - Validation support for custom types
- **Auto-gitignore on init**
  - `co init <name>` automatically adds space to `.gitignore`
  - Prevents accidental commits of user spaces to co home

### Fixed
- Language validation now accepts known languages (english, portuguese, etc.) without requiring directory
- Content type pluralization: `user-story` → `user-stories/` (not `user-storys/`)
- Clippy warnings resolved for CI compliance (#46)

## [0.11.0] - 2026-01-03

### Added
- **Work Item Types & Content Parsing** (#33, #34)
  - User-story sections: `## As`, `## I Need`, `## To`
  - Task sections: `## Given`, `## When`, `## Then`
  - Built-in types: `user-story`, `task`, `epic`, `release`
  - Content section validation for structured formats
  - `work/schema.yaml` for work item type definitions

## [0.10.0] - 2026-01-03

### Added
- **Feature System** (#31)
  - Automatic discovery of `agents/` and `tools/` directories
  - Schema-based content type registration via `schema.yaml`
  - Feature registry for extensibility
  - `co config show` displays discovered features

### Fixed
- Version updated to 0.10.1 with UI reorganization (#32)

## [0.9.0] - 2026-01-02

### Added
- **Interactive REPL** (#28)
  - `co lead` - Interactive exploration mode
  - Commands: `status`, `locate`, `use <scope>`, `help`, `quit`
  - Scope-aware prompts
  - Real-time content navigation

## [0.6.0] - 2026-01-02

### Added
- **Validation System** (#27)
  - `co validate <item>` - Validate specific content
  - `co validate all` - Validate entire workspace
  - Frontmatter validation (required fields, types)
  - Internal link validation (`[[references]]`)
  - Language and scope existence checks
  - Severity levels: Error, Warning

## [0.5.0] - 2026-01-02

### Added
- **Index & Performance** (#25)
  - SQLite-based content indexing
  - `co locate build` - Build/rebuild index
  - `co locate --stats` - Show index statistics
  - Incremental index updates (only modified files)
  - Full-text search via FTS5

### Fixed
- Deprecated exports removed, CI workflow fixed (#26)

## [0.4.0] - 2026-01-02

### Added
- **Query System** (#23)
  - `co locate` - Unified search command
  - Filter by type: `co locate --type task`
  - Filter by scope: `co locate --scope private`
  - Full-text search: `co locate "search term"`
  - Combined filters and search

### Changed
- Unified `find` and `search` into single `co locate` command (#24)

## [0.3.0] - 2026-01-02

### Added
- **Content Management** (#22)
  - `co new <type> <name>` - Create new content
  - `co show <item>` - Display content
  - `co update <item> --status <status>` - Update metadata
  - `co delete <item>` - Remove content
  - Frontmatter parsing with YAML support
  - Content type detection

## [0.2.0] - 2026-01-02

### Added
- **Language Foundations** (#21)
  - Multi-language support (english, portuguese, guarani-mbya)
  - Internationalization (i18n) for CLI messages
  - `co lang <code>` - Set UI language
  - `co languages` - List supported languages
  - Lexicon structure for definitions
  - Language-specific directories (`en/`, `pt/`, `gun/`)

## [0.1.0] - 2026-01-02

### Added
- Initial release
- Graph-based content management foundation
- `co init <name>` - Initialize context
- `co list` - List contexts and languages
- `co status` - Show workspace status
- Basic CLI structure with clap
- Workspace configuration (`.co/config.yaml`)

---

## Roadmap

### Upcoming (v1.0)
- [x] #36 - GitHub as Source of Truth (sync issues/PRs)
- [x] #38 - Plan & Execute Workflow
- [x] #39 - Writer Agent System
- [x] #40 - Tools & Extensions
- [x] #41 - Analyze Command
- [ ] #42 - Documentation Polish
- [x] #43 - Archive & Storage
- [x] #47 - Space Isolation & Commit Guards
- [x] #48 - Collaborative Content Creation (User + Agent)
- [x] #49 - Terminology Refactor (space/context/scope)
