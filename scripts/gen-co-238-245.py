#!/usr/bin/env python3
"""Generate CO-238..245 task specs for the additions raised during 2.9.0 release."""
from pathlib import Path

CO = Path("/Users/artelonga/projects/co/work/co")
CREATED = "2026-05-18T00:00:00Z"

def labels_block(items):
    return "\n".join(f"  - {l}" for l in items)

def render(*, id, title, commit, semver, priority, module, labels, parent,
           role, need, so_that, principles, scope, acceptance, blast):
    return f"""---
id: {id}
title: "{title}"
type: user-story
status: todo
priority: {priority}
conventional_commit: "{commit}"
semver_bump: {semver}
labels:
{labels_block(labels)}
module: {module}
parent: {parent}
created_at: {CREATED}
updated_at: {CREATED}
---

## As

{role}

## I Need

{need}

## So That

{so_that}

## Context

- **Principles:** {principles}
- **Scope:** {scope}

## Acceptance

{acceptance}

## Blast radius

{blast}
"""

def write(id, content):
    p = CO / f"CO-{id}.md"
    p.write_text(content)
    print(f"  wrote {p}")

# CO-238 — Sidebar UX clarity
write(238, render(
    id=238, parent=231,
    title="Sidebar UX — clarify owned/member/role/sub-universe semantics",
    commit="refactor(sidebar):", semver="patch",
    priority="medium", module="co-web",
    labels=["type:refactor", "module:sidebar", "module:ux"],
    role="A user opening the universe sidebar",
    need="Self-explanatory section labels, visible ownership chips, sub-universe counts, and tooltips clarifying the relationship for each section",
    so_that="The distinction between universes I created vs am a member of vs subscribe to vs can discover is immediately legible, not inferred from the absence of a badge.",
    principles="§6 (folders/grouping encapsulate meaning), §3 (static signaling)",
    scope=(
        "Today `co-web/static/variants/a/modules/sidebar.js:112-153` renders three sections from `me.owned`, `me.member`, "
        "`me.subscribed`. The first calls `renderSectionHtml(..., me.owned, false)` (no role chip) — ownership is implicit. "
        "The screenshot user-report shows confusion: \"why does ArteLonga have no badge but Quilombo Araucária has admin\".\n\n"
        "Changes:\n"
        "1. Rename labels (i18n): `sidebar.section.owned` 'Meus universos' → 'Universos que criei'; `sidebar.section.member` "
        "'Comunidades' → 'Universos onde tenho papel' (or similar — pick the cleanest).\n"
        "2. Show a chip on owned universes too (e.g. 'criador') so the visual treatment is consistent.\n"
        "3. Surface sub-universe count next to each parent (e.g. 'ArteLonga (3 sub)').\n"
        "4. Add a hover tooltip per section label explaining the relationship.\n"
        "5. Render sub-universes across section bucket boundaries: if `tempo.parent_key='template'` and the user is anonymous "
        "to template, still show tempo nested under a synthetic 'template' parent in 'Descobrir'."
    ),
    acceptance=(
        "- Renamed labels live in `co-web/static/shared/i18n.js` for both pt and en variants.\n"
        "- Owned universes show a chip (text 'criador' or icon, pick one in review).\n"
        "- Sub-universe count appears next to parent (`(N)` suffix).\n"
        "- Hover tooltip on `.sidebar-section-label` shows section semantics.\n"
        "- `buildChildMap` extended to render synthetic parents when parent_key references a universe not in the user's bucket.\n"
        "- Playwright test: sign in as a user with at least 2 owned + 1 admin-member + 1 subscribed; assert every section "
        "renders the correct label and badge state."
    ),
    blast="Small — JS module + i18n strings + 1 CSS rule. No server-side change.",
))

# CO-239 — Real host disk stats
write(239, render(
    id=239, parent=231,
    title="Fix host stats — wire nix::sys::statvfs for data_dir_total / data_dir_available",
    commit="fix(storage):", semver="patch",
    priority="low", module="co-web",
    labels=["type:fix", "module:storage", "module:observability"],
    role="An operator or admin checking `/storage` to forecast capacity",
    need="Real values for `data_dir_total_bytes` and `data_dir_available_bytes`",
    so_that="I can see how much disk is left on the Fly volume before I need to expand it, instead of zero placeholders.",
    principles="§3 (static typing — values must reflect reality)",
    scope=(
        "`co-web/src/storage_dashboard.rs:114-122` returns hardcoded zeros for `data_dir_total_bytes` and "
        "`data_dir_available_bytes`. The comment says: 'statfs-based total/available requires a libc binding we don't currently "
        "depend on; skipped here.'\n\n"
        "Add `nix = { version = \"0.x\", features = [\"fs\"] }` and call `nix::sys::statvfs::statvfs(data_dir)`. Compute:\n"
        "- `total_bytes = f_blocks * f_frsize`\n"
        "- `available_bytes = f_bavail * f_frsize`\n\n"
        "Or use `libc::statvfs64` direct (no extra dep) if we want to keep the dependency surface minimal."
    ),
    acceptance=(
        "- `/api/v1/admin/storage` returns non-zero values for both fields on a real volume.\n"
        "- Linux-only (gated by `#[cfg(target_os = \"linux\")]`); macOS/Windows return None or zero with a `tracing::warn!`.\n"
        "- Integration test against a tmpfs mount asserts total > used > 0."
    ),
    blast="Tiny — one function + optional dep.",
))

# CO-240 — Per-universe data_db_bytes fix
write(240, render(
    id=240, parent=231,
    title="Fix per-universe data_db_bytes — currently 0 for every universe",
    commit="fix(storage):", semver="patch",
    priority="medium", module="co-web",
    labels=["type:fix", "module:storage", "module:observability"],
    role="An operator examining per-universe storage on `/storage`",
    need="`data_db_bytes` to reflect the actual size of each universe's data.db SQLite file",
    so_that="I can identify the largest universes to prioritize backup, replication, or compaction.",
    principles="§3 (typing — values reflect reality)",
    scope=(
        "API response shows `data_db_bytes: 0` for every universe — ArteLonga (312 entries), CO (877 entries), "
        "comunicacao (4692 entries), etc.\n\n"
        "Trace `co-web/src/storage_dashboard.rs:205` `file_size(&data_db_path)`. The `data_db_path` construction likely "
        "doesn't match where the `UniversePool` actually opens the per-universe SQLite. Confirm path via:\n\n"
        "```rust\n"
        "let universe_dir = storage.universe_root(key); // e.g. /data/universes/artelonga\n"
        "let data_db = universe_dir.join(\"data.db\");\n"
        "```\n\n"
        "If the file genuinely doesn't exist yet (universe-level DB lazily created on first write), document that and return "
        "`Some(0)` with a flag distinguishing 'not yet materialised' from 'broken'."
    ),
    acceptance=(
        "- After creating a universe + 5 entries via vault PUT, `data_db_bytes > 0` in the API response.\n"
        "- Integration test in `tests/` covers create-universe → put-entry → assert non-zero.\n"
        "- Sum of per-universe `data_db_bytes` matches `du -sb /data/universes/*/data.db` (or close, with FS block alignment)."
    ),
    blast="Tiny — path construction + test.",
))

# CO-241 — True content-volume metrics (lines/words/chars)
write(241, render(
    id=241, parent=231,
    title="Add true content-volume metrics (lines, words, chars) — fix 'lines = files' confusion",
    commit="feat(stats):", semver="minor",
    priority="medium", module="co-web",
    labels=["type:feat", "module:storage", "module:observability"],
    role="A user looking at the storage dashboard or universe overview",
    need="A real lines/words/chars count of all .md content per universe, separate from `content_count` (file count)",
    so_that="I see at a glance how much *content* my universe holds, not just how many files it has — and the dashboard never mislabels 'Entradas' as 'lines'.",
    principles="§3 (static typing — metrics named for what they measure), §6 (data fields encapsulate one meaning)",
    scope=(
        "Add three new columns to `entries`:\n\n"
        "```sql\n"
        "ALTER TABLE entries ADD COLUMN body_lines INTEGER NOT NULL DEFAULT 0;\n"
        "ALTER TABLE entries ADD COLUMN body_words INTEGER NOT NULL DEFAULT 0;\n"
        "ALTER TABLE entries ADD COLUMN body_chars INTEGER NOT NULL DEFAULT 0;\n"
        "```\n\n"
        "Compute on insert/update in vault PUT handler:\n"
        "- `lines = body.lines().count()`\n"
        "- `words = body.split_whitespace().count()`\n"
        "- `chars = body.chars().count()`\n\n"
        "Aggregate per-universe in the dashboard response:\n"
        "```rust\n"
        "pub struct UniverseStats {\n"
        "    pub content_count: i64,    // existing\n"
        "    pub md_bytes: u64,         // existing\n"
        "    pub body_lines: i64,       // new\n"
        "    pub body_words: i64,       // new\n"
        "    pub body_chars: i64,       // new\n"
        "    ...\n"
        "}\n"
        "```\n\n"
        "Update `co-web/static/shared/storage.html` to surface the new fields. NEVER use the label 'lines' for `content_count` — "
        "label that field as 'Entradas' / 'Entries' uniformly.\n\n"
        "Migration: a one-time backfill pass on boot that walks the entries table and computes the three counts for rows with "
        "DEFAULT 0 values. Idempotent — re-running is a no-op."
    ),
    acceptance=(
        "- Migration adds the three columns.\n"
        "- Vault PUT writes the three counts on every insert/update.\n"
        "- `/api/v1/admin/storage` returns non-zero `body_lines` aggregates per universe.\n"
        "- The storage HTML page shows 'Entradas / Linhas / Palavras / Tamanho' columns distinctly.\n"
        "- Audit script: `grep -r '\"lines\"' co-web/static` returns nothing where `content_count` is rendered."
    ),
    blast="Medium — schema migration + UI labels. Backfill is bounded by total entries (~7000 across all universes today).",
))

# CO-242 — Unified file listing (all file types)
write(242, render(
    id=242, parent=231,
    title="Unified file listing — surface all file types in universe entries (PDF, image, video, code)",
    commit="feat(content):", semver="minor",
    priority="medium", module="co-web",
    labels=["type:feat", "module:content", "module:assets"],
    role="A user browsing a universe",
    need="All files I've uploaded — .md, PDFs, images, videos, code — visible in the same unified tree, not split between 'entries' and 'assets'",
    so_that="A universe really is a single home for all my content, not a markdown-only space with assets relegated to a separate API.",
    principles="§6 (folders encapsulate features — every file under one model), §1 (composition — assets become a kind of entry)",
    scope=(
        "Today entries (`GET /api/v1/universes/{slug}/entries`) returns rows from the `entries` table only. Assets "
        "(CO-145/146/147) live in a separate `assets` table per universe, accessed via `/assets/<sha>` endpoints.\n\n"
        "Merge the two listings:\n"
        "- Add an `entries` row for every asset upload (entry_type = `'asset.pdf' | 'asset.image' | 'asset.video' | "
        "'asset.code' | 'asset.binary'`).\n"
        "- The row stores the path (e.g. `attachments/foo.pdf`), mime type, asset_sha256 reference, and ts.\n"
        "- The body column is empty/short metadata; the actual bytes stay in the `assets` table (deduplicated by SHA-256).\n"
        "- Renderer dispatches by entry_type:\n"
        "  - `page`, `task`, etc. → existing markdown view\n"
        "  - `asset.pdf` → embed pdf.js viewer\n"
        "  - `asset.image` → `<img>` with the encrypted-decrypt streaming endpoint\n"
        "  - `asset.video` → `<video>` with HLS or direct mp4\n"
        "  - `asset.code` → CodeMirror read-only view\n\n"
        "Phase 1 (this task): backfill entries rows for existing assets; surface in entries API + tree; renderer supports "
        "PDF + image + video + code-read-only.\n\n"
        "Phase 2 (CO-245): inline editing for plaintext file types via CodeMirror."
    ),
    acceptance=(
        "- New migration creates entries rows for each existing assets row (paths under `attachments/`).\n"
        "- Vault PUT accepts binary uploads as first-class entries; creates assets row + entries row in one transaction.\n"
        "- `GET /api/v1/universes/{slug}/entries` returns asset entries by default (filterable via `?type=asset.*`).\n"
        "- Frontend renders PDF (pdf.js), image, video, and code (CodeMirror RO) when clicking an asset entry.\n"
        "- `content_count` aggregates across both — universe page count increases after asset upload."
    ),
    blast="Large — adds asset upload to entries flow + 4 renderer dispatches + 1 migration. Phase 1 only; editing comes in CO-245.",
))

# CO-243 — VS Code / Zed / Helix integration
write(243, render(
    id=243, parent=231,
    title="VS Code (and LSP) integration — open universe as remote workspace",
    commit="feat(integrations):", semver="minor",
    priority="low", module="co-web",
    labels=["type:feat", "module:integrations", "cross-repo:universal-template"],
    role="A developer editing CO universe content from VS Code, Neovim, Helix, or Zed",
    need="An extension or LSP server that exposes the universe's content as a virtual file system (or LSP workspace) so my editor talks to the CO Vault API natively",
    so_that="I can use my regular editor + extensions (linters, AI assistants, formatters) on universe content without leaving the tool, and CO's vault stays the source of truth.",
    principles="§4 (reduced coupling — same content, multiple front-ends)",
    scope=(
        "Three viable integration approaches; pick one (or two complementary):\n\n"
        "**Option A — VS Code extension** (recommended for Phase 1):\n"
        "Register CO as a remote workspace via `vscode.workspace.registerFileSystemProvider`. The extension:\n"
        "- Logs in (re-uses the API token CLI from CO-236)\n"
        "- Lists universes the user can access\n"
        "- 'Open Remote Workspace' shows the entries tree as files\n"
        "- Read = vault GET, write = vault PUT\n"
        "- File metadata (mtime, size) maps to entry frontmatter\n\n"
        "**Option B — LSP server** (Phase 2 — broad editor reach):\n"
        "A `co-lsp` binary that any LSP-aware editor can launch. Implements:\n"
        "- File completion for wikilinks (`[[...]]`)\n"
        "- Hover for cross-entry references\n"
        "- Definition for entry IDs\n"
        "- Diagnostics for broken links + missing frontmatter\n"
        "Works in VS Code, Neovim, Helix, Zed, Emacs Eglot.\n\n"
        "**Option C — WebDAV mount** (Phase 3 — every editor):\n"
        "Expose Vault API via WebDAV. User does `mount -t davfs https://co.artelonga.com.br/dav/<slug> /mnt/co`. Heaviest "
        "(needs auth proxy, locking semantics) but works in every editor including emacs/vim/Sublime."
    ),
    acceptance=(
        "Phase 1 (Option A):\n"
        "- `co-vscode` extension published to the marketplace (or sideloadable VSIX).\n"
        "- After install + auth, 'Open Remote → CO' lists user's universes.\n"
        "- Edit a .md file in VS Code, save → vault PUT succeeds, CO web shows the change.\n"
        "- Optional: auto-completion of wikilinks from the entries index.\n\n"
        "Phase 2 (Option B):\n"
        "- `co-lsp` binary, configurable per-editor.\n"
        "- Tested in at least VS Code + Neovim."
    ),
    blast="Medium — separate repo (co-vscode) for the extension; doesn't touch co-web. LSP is its own crate.",
))

# CO-244 — Python / R REPL interoperability
write(244, render(
    id=244, parent=231,
    title="Python / R REPL interoperability — DuckDB attach + in-browser REPL",
    commit="feat(integrations):", semver="minor",
    priority="low", module="co-web",
    labels=["type:feat", "module:integrations", "cross-repo:universal-template"],
    role="A researcher or analyst querying universe content from Python or R",
    need="A frictionless path to query a universe's entries / assets / events / relations from Python or R, with full SQL + DataFrame ergonomics",
    so_that="I can do ad-hoc analysis (visualizations, ML, batch transforms) on universe data without writing API clients — the per-universe SQLite is already the right shape.",
    principles="§4 (reduced coupling — data accessible without app intermediation)",
    scope=(
        "**Approach (zero new server code needed):** DuckDB attaches SQLite directly. From Python or R:\n\n"
        "```python\n"
        "import duckdb\n"
        "con = duckdb.connect()\n"
        "con.execute(\"ATTACH '/data/universes/artelonga/data.db' AS al (READ_ONLY)\")\n"
        "df = con.execute(\"\"\"\n"
        "    SELECT path, body_lines, updated_at, frontmatter_json::JSON->>'status' as status\n"
        "    FROM al.entries WHERE entry_type='task' AND status='done'\n"
        "    ORDER BY updated_at DESC LIMIT 100\n"
        "\"\"\").df()\n"
        "```\n\n"
        "Same code in R via the `duckdb` package.\n\n"
        "**Deliverables:**\n"
        "1. A `co-py` Python helper package (~50 LOC) that:\n"
        "   - Resolves universe slug → local data.db path (or downloads a snapshot via API)\n"
        "   - Opens read-only DuckDB connection\n"
        "   - Returns a connection ready for queries\n"
        "2. A `co-r` R helper package (mirror of co-py).\n"
        "3. (Phase 2) In-browser REPL: Pyodide + DuckDB-WASM running against an in-memory copy of the universe's data.db. "
        "Embedded as a `/repl` panel in the CO SPA. User writes Python in-page, queries their own data, no server roundtrip.\n"
        "4. (Phase 2) Jupyter kernel `%co_query` magic that wraps the above.\n\n"
        "**For hosted / read-only access** (when user doesn't have local data.db): add a `POST /api/v1/universes/{slug}/query` "
        "endpoint that accepts a SQL string, runs it against the universe DB read-only (with row-count + execution-time cap), "
        "returns results as JSON. Python/R packages fall back to this when the SQLite isn't local."
    ),
    acceptance=(
        "Phase 1:\n"
        "- `pip install co-py` + 3 lines of Python returns a working DuckDB connection to a universe.\n"
        "- Same in R via `install.packages('co-r')`.\n"
        "- Example notebook ships in repo: 'Querying CO universes with DuckDB'.\n\n"
        "Phase 2:\n"
        "- In-browser REPL at `/repl?u=<universe>` runs Pyodide queries against the universe.\n"
        "- `POST /api/v1/universes/{slug}/query` available for hosted access (with auth + read-only)."
    ),
    blast="Small — packaging work, no server changes for Phase 1. Phase 2 adds a query endpoint + WASM bundle.",
))

# CO-245 — Embedded code editor for non-md files
write(245, render(
    id=245, parent=231,
    title="Inline code editor for plaintext file types (CodeMirror)",
    commit="feat(editor):", semver="minor",
    priority="low", module="co-web",
    labels=["type:feat", "module:editor", "module:content"],
    role="A user editing code, YAML, JSON, CSV, or similar plaintext files stored in a universe",
    need="Inline editing of these file types in the CO web view, with syntax highlighting and save-back to vault",
    so_that="I don't have to download → edit locally → upload — I can iterate on a config file or script directly in the universe.",
    principles="§4 (reduced coupling — editor lives next to content), §6 (universal substrate for any plaintext file)",
    scope=(
        "After CO-242 surfaces all file types in entries, add edit affordance for plaintext types:\n\n"
        "Supported languages (Phase 1, via CodeMirror's existing modes — already bundled at `static/shared/editor.bundle.js`):\n"
        "- `code.rs` Rust\n"
        "- `code.ts/.js` TypeScript / JavaScript\n"
        "- `code.py` Python\n"
        "- `code.r` R\n"
        "- `code.sh` Bash\n"
        "- `code.sql` SQL\n"
        "- `code.go` Go\n"
        "- `code.yaml/.yml` YAML\n"
        "- `code.json` JSON\n"
        "- `code.toml` TOML\n"
        "- `text.csv/.tsv` CSV (with table view toggle)\n\n"
        "Phase 2: inline run/preview for some types — e.g. CSV opens a sortable/filterable table view; YAML lints against a "
        "schema if the universe declares one.\n\n"
        "Phase 3: a 'CO Pad' mode — non-md entries shown side-by-side with their preview (markdown ↔ rendered, CSV ↔ table, "
        "etc.)."
    ),
    acceptance=(
        "- Clicking a code/yaml/json/csv entry opens a CodeMirror editor with appropriate syntax highlighting.\n"
        "- Save button writes back via vault PUT, increments `body_lines/words/chars` (CO-241).\n"
        "- ctrl/cmd+S keybind works.\n"
        "- Read-only fallback for unknown plaintext types.\n"
        "- E2E test: upload a `.py` file, edit it inline, save, reload, content matches."
    ),
    blast="Medium — frontend only; CodeMirror is already bundled. New 'inline editor' view + dispatcher in the entry router.",
))

print("\nDone. Wrote 8 specs (CO-238..245).")
