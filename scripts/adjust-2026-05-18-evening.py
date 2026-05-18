#!/usr/bin/env python3
"""Adjustments after evening review (2026-05-18):

1. Renumber my QB-1..15 specs (currently in quilombo-blog) → QB-14..28 in
   quilomboaraucaria (the canonical content+code repo). Quilombo-blog's
   audit history stays (merged PR #2); quilomboaraucaria becomes
   authoritative going forward.

2. Add YG-51 (port Godot games from universos), YG-52 (reconcile game-core
   drift), YG-53 (archive universos) to yggdrasil's backlog.

3. Add CO-247 (Fizzy compatibility for CO's quadro / kanban) to CO.

4. Sync project.yaml next_id values where they're stale.
"""
import re
from pathlib import Path

QB_SRC = Path("/Users/artelonga/projects/quilombo-blog/work/qb")
QB_DST = Path("/Users/artelonga/projects/quilomboaraucaria/work/qb")
YG     = Path("/Users/artelonga/projects/yggdrasil/work/yggdrasil")
CO     = Path("/Users/artelonga/projects/co/work/co")
CREATED = "2026-05-18T00:00:00Z"

ID_SHIFT = 13   # my QB-1 → QB-14; my QB-13 (epic) → QB-26

def write(path: Path, content: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    print(f"  wrote {path}")

def labels_block(items):
    return "\n".join(f"  - {l}" for l in items)

# ============================================================
# 1. QB renumber — move from quilombo-blog to quilomboaraucaria
# ============================================================
def shift_qb():
    print("\n=== QB renumber: quilombo-blog → quilomboaraucaria (+13) ===")
    for n in range(1, 16):  # QB-1..15 inclusive
        src = QB_SRC / f"QB-{n}.md"
        if not src.exists():
            print(f"  SKIP QB-{n} (not in source)")
            continue
        new_id = n + ID_SHIFT
        text = src.read_text()
        # Shift the id: field
        text = re.sub(r"^id:\s*\d+\s*$", f"id: {new_id}", text, count=1, flags=re.M)
        # Shift the title (no — title text stays semantic)
        # Shift parent: if 13/14/15
        def shift_parent(m):
            old = int(m.group(1))
            if old in (13, 14, 15):
                return f"parent: {old + ID_SHIFT}"
            return m.group(0)
        text = re.sub(r"^parent:\s*(\d+)\s*$", shift_parent, text, count=1, flags=re.M)
        # Add a note in the frontmatter that this was migrated
        text = text.replace(
            f"updated_at: {CREATED}",
            f"updated_at: {CREATED}\nmigrated_from: quilombo-blog/work/qb/QB-{n}.md",
        )
        dst = QB_DST / f"QB-{new_id}.md"
        write(dst, text)

    # bump next_id 13 → 29 in quilomboaraucaria
    pyaml = QB_DST / "project.yaml"
    if pyaml.exists():
        t = pyaml.read_text()
        t = re.sub(r"^next_id:\s*\d+\s*$", "next_id: 29", t, flags=re.M)
        pyaml.write_text(t)
        print(f"  bumped {pyaml} next_id → 29")

# ============================================================
# 2. YG-51, 52, 53 — yggdrasil compatibility tasks
# ============================================================
def render_yg_story(*, id, parent, title, commit, semver, priority, labels,
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
module: yggdrasil
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

def gen_yg_compat():
    print("\n=== YG-51/52/53: universos compatibility ===")
    write(YG / "YG-51.md", render_yg_story(
        id=51, parent=48,
        title="Port Godot games from universos into yggdrasil-godot",
        commit="feat(godot):", semver="minor",
        priority="high",
        labels=["type:feat", "module:godot", "cross-repo:universos-bridge"],
        role="A player launching the Godot client to play snake / tetris / invaders / poker / pointset",
        need="The Godot scenes + GDScript implementations from universos/godot ported into yggdrasil/yggdrasil-godot",
        so_that="The yggdrasil-godot subproject (currently 9 files — just the HelloUniverso placeholder) gains real game implementations, matching what the Rust route handlers in yggdrasil-web/src/games/ already expose.",
        principles="§6 (folders encapsulate features), §4 (single canonical home for game client)",
        scope=(
            "Today `yggdrasil-godot/` has 9 files (a HelloUniverso template). `universos/godot/` has 54 files including:\n\n"
            "- `scenes/games/{snake, tetris, invaders, pointset, poker}/` (full scenes per game)\n"
            "- `scripts/games/{snake, tetris, invaders, pointset, poker}/` (GDScript implementations)\n"
            "- `scenes/lobby/`, `scripts/lobby/` (lobby scene + grid + portals)\n"
            "- `scenes/main.tscn`, `scripts/autoloads/` (entry point + autoloads)\n\n"
            "Port the games + lobby into yggdrasil-godot. Each game's GDScript talks to the Rust HTTP routes in "
            "yggdrasil-web (snake_routes.rs, tetris_routes.rs, etc.) — the contract already exists."
        ),
        acceptance=(
            "- All 5 games render in yggdrasil-godot client (`make run` or equivalent).\n"
            "- Each game's GDScript hits the correct yggdrasil-web route — e.g. snake POSTs to `/api/v1/games/snake/start` and ticks via `/api/v1/games/snake/tick`.\n"
            "- Lobby scene loads + portals to each game.\n"
            "- E2E smoke test: launch Godot client → play 1 round of each game → score persists to scores table.\n"
            "- `universos/godot/` no longer referenced anywhere in active code."
        ),
        blast="Medium — ~45 files added (scenes + scripts). No backend changes; reuses existing Rust route contract.",
    ))

    write(YG / "YG-52.md", render_yg_story(
        id=52, parent=49,
        title="Reconcile game-core drift between universos/core and co/game-core",
        commit="fix(game-core):", semver="patch",
        priority="high",
        labels=["type:fix", "module:game-core", "cross-repo:universos-bridge"],
        role="A maintainer of yggdrasil + co/game-core (pinned via YG-38)",
        need="The diverged crates `universos/core` and `co/game-core` reconciled into a single canonical `co/game-core`",
        so_that="YG-38 pins yggdrasil to `co/game-core` at a known good rev (currently 268ea54). Anything still unique to universos's copy (notably `universo.rs` + improvements to the shared 49a25f9f/*.rs files) is silently lost. This reconciles, then advances the YG-38 pin.",
        principles="§4 (single source of truth — one canonical crate, not two)",
        scope=(
            "Files in universos/core/src/ NOT in co/game-core/src/:\n"
            "- `universo.rs` — review + port if non-trivial\n\n"
            "Files in co/game-core/src/ NOT in universos/core/src/:\n"
            "- `mail.rs` — keep (newer)\n\n"
            "Shared files that DIFFER between the two repos:\n"
            "- `49a25f9f/04f8996d.rs`, `49a25f9f/3549b002.rs`, `49a25f9f/8a6cead4.rs`, `49a25f9f/mod.rs`\n"
            "- `lib.rs`, `plugin.rs`\n\n"
            "Process:\n"
            "1. `diff -u universos/core/src/<file> co/game-core/src/<file>` per shared file. Identify universos-side "
            "improvements that should be preserved (heuristic: if universos has bug fixes / refactors that co/game-core "
            "lacks, port them).\n"
            "2. Port `universos/core/src/universo.rs` into `co/game-core/src/universo.rs` (or fold its contents into an "
            "existing module if it duplicates).\n"
            "3. Tag co/game-core 0.2.0 after the merge.\n"
            "4. Advance yggdrasil's pinned rev in `yggdrasil/Cargo.toml` (currently `rev = \"268ea54...\"`) to the new "
            "co/game-core 0.2.0 commit."
        ),
        acceptance=(
            "- `diff -rq universos/core/src co/game-core/src` returns only `Only in co/game-core/src: mail.rs` (no diffs in shared files).\n"
            "- `co/game-core` tagged 0.2.0.\n"
            "- yggdrasil/Cargo.toml advances pinned rev; `cargo test --workspace` still passes.\n"
            "- CHANGELOG entries in both co and yggdrasil note the reconciliation."
        ),
        blast="Medium — touches every diverged file in co/game-core; potential test churn if universo.rs introduces new types.",
    ))

    write(YG / "YG-53.md", render_yg_story(
        id=53, parent=49,
        title="Archive universos repo after YG-51 + YG-52 land",
        commit="chore(deprecation):", semver="patch",
        priority="low",
        labels=["type:chore", "module:deprecation"],
        role="A maintainer keeping the repo set clean",
        need="The universos repo archived once YG-51 (Godot games ported) and YG-52 (game-core reconciled) ship",
        so_that="There's a single canonical home for the game stack (yggdrasil + co/game-core), not two diverging forks.",
        principles="§4 (reduced coupling — no parallel canonicals)",
        scope=(
            "Once YG-51 + YG-52 are merged:\n"
            "1. Add a top-level `ARCHIVED.md` to universos explaining the merge into yggdrasil + co/game-core, with pointers.\n"
            "2. Rename the GitHub repo to `artelonga/universos-archive`.\n"
            "3. Set the repo description to 'Archived — see artelonga/yggdrasil and artelonga/co/game-core'.\n"
            "4. Mark all open issues / PRs (if any) closed with a comment pointing to the canonical replacement.\n"
            "5. Optional: delete the local checkout at `~/projects/universos/` (data is on GitHub if needed for recovery)."
        ),
        acceptance=(
            "- `gh repo view artelonga/universos` shows description 'Archived — see artelonga/yggdrasil and artelonga/co/game-core'.\n"
            "- README.md (or ARCHIVED.md) at the repo root has a clear pointer.\n"
            "- No active CI / deploy hooks remain.\n"
            "- yggdrasil's own CLAUDE.md updated to remove any reference to universos as a sibling."
        ),
        blast="Zero — operational + docs only.",
    ))

    # bump yg next_id 38 → 54
    pyaml = YG / "project.yaml"
    t = pyaml.read_text()
    t = re.sub(r"^next_id:\s*\d+\s*$", "next_id: 54", t, flags=re.M)
    pyaml.write_text(t)
    print(f"  bumped {pyaml} next_id → 54")

# ============================================================
# 3. CO-247 — Fizzy (basecamp) compatibility for CO quadro
# ============================================================
def gen_co_fizzy():
    print("\n=== CO-247: Fizzy compatibility ===")
    body = f"""---
id: 247
title: "Fizzy compatibility for CO quadro — import / export / shared schema"
type: user-story
status: todo
priority: medium
conventional_commit: "feat(quadro):"
semver_bump: minor
labels:
  - type:feat
  - module:quadro
  - module:integrations
  - cross-repo:universal-template
module: co-web
parent: 231
created_at: {CREATED}
updated_at: {CREATED}
---

## As

A team using both CO and Basecamp's Fizzy (https://github.com/basecamp/fizzy) for kanban

## I Need

Two-way data interop between CO's `quadro` view and Fizzy boards: import a Fizzy export into a CO universe; export a CO board into a Fizzy-compatible format

## So That

Teams aren't forced to pick one tool; CO boards inherit Fizzy's well-known kanban UX vocabulary; teams can migrate either direction without manual copy-paste.

## Context

- **Principles:** §4 (reduced coupling — open data contracts), §3 (static typing — board entries map to a known external schema)
- **What is Fizzy:** Basecamp's open-source kanban ("Kanban as it should be"). Ruby on Rails app, 7700+ stars. Simple model: lists (columns) ← cards ← optional checklists + comments.
- **CO's analog:** the `quadro` view renders entries with `type: task` (or similar) grouped by `status` column. Each card has title, status, priority, labels, assignee, dates.

## Scope

**Phase 1 — schema mapping doc** (`docs/integrations/fizzy.md`):

| Fizzy concept | CO concept |
|---|---|
| Board | Universe (or a project within a universe) |
| List / column | Status value (`todo` / `in_progress` / `done` / custom) |
| Card | Entry of `type: task` |
| Card title | `frontmatter.title` |
| Card description | Markdown body |
| Card assignee | `frontmatter.assignee` |
| Card due date | `frontmatter.due_at` (semantic) |
| Card labels | `frontmatter.labels` (array) |
| Card checklist | A `## Checklist` section in the body OR child entries with `type: subtask` |
| Card comments | Replies via the chat / commentary subsystem (CO-195+) |

**Phase 2 — import**:
- `POST /api/v1/universes/{{slug}}/quadro/import?source=fizzy` accepting a Fizzy export JSON. Creates entries 1:1.
- CLI: `co quadro import --source fizzy <path.json>`.

**Phase 3 — export**:
- `GET /api/v1/universes/{{slug}}/quadro/export?format=fizzy` returning Fizzy-shaped JSON.
- CLI: `co quadro export --format fizzy > out.json`.

**Phase 4 — live mirror (optional)**:
- A worker that syncs CO entry changes to a Fizzy board via Fizzy's HTTP API (if a remote Fizzy instance is configured).
- Bi-directional via webhooks if Fizzy exposes them.

## Acceptance

Phase 1:
- `docs/integrations/fizzy.md` documents the schema mapping (the table above) + concrete examples (one CO entry frontmatter ↔ one Fizzy card JSON).
- Mapping reviewed against an actual Fizzy export file.

Phase 2:
- `POST /api/v1/universes/{{slug}}/quadro/import?source=fizzy` accepts the export, creates entries.
- Round-trip preserves: title, status, body, assignee, due_at, labels.
- Integration test imports a 10-card Fizzy fixture and asserts each card became an entry.

Phase 3:
- `GET /api/v1/universes/{{slug}}/quadro/export?format=fizzy` returns Fizzy-shaped JSON.
- Round-trip: import → export → import again preserves data (no information loss).

Phase 4 (deferred): tracked as CO-N follow-up.

## Blast radius

Phase 1 docs-only. Phase 2+3 add 2 routes + a small schema mapper module. No changes to the entries table.

## Related

- **CO-244** — Python/R REPL via DuckDB attach; the Fizzy export query becomes a one-liner.
- **CO-242** — Unified file listing; a Fizzy board snapshot could be an entry of `type: asset.fizzy.json`.
"""
    write(CO / "CO-247.md", body)
    # bump co next_id 246 → 248
    pyaml = CO / "project.yaml"
    t = pyaml.read_text()
    t = re.sub(r"^next_id:\s*\d+\s*$", "next_id: 248", t, flags=re.M)
    pyaml.write_text(t)
    print(f"  bumped {pyaml} next_id → 248")

if __name__ == "__main__":
    shift_qb()
    gen_yg_compat()
    gen_co_fizzy()
    print("\nDone.")
