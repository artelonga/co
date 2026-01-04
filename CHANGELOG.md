# Changelog

All notable changes to CO are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
- [ ] #40 - Tools & Extensions
- [ ] #41 - Analyze Command
- [ ] #42 - Documentation Polish
- [ ] #43 - Archive & Storage
- [x] #47 - Space Isolation & Commit Guards
- [x] #48 - Collaborative Content Creation (User + Agent)
- [x] #49 - Terminology Refactor (space/context/scope)
