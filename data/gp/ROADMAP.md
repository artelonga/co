# GP Roadmap — Execution Order for `co auto`

## Release Plan (Semver Milestones)

### v0.1.0 — Base App Foundation
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 1 | GP-2: Rust workspace structure | core, server | `feat(core)` | — |
| 2 | GP-3: Plugin trait + manifest | core | `feat(core)` | GP-2 |
| 3 | GP-4: Plugin loader + registration | server | `feat(server)` | GP-3 |

### v0.2.0 — Auth + Web Frontend
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 4 | GP-6: User registration endpoint | server | `feat(server)` | GP-2 |
| 5 | GP-7: Login endpoint + JWT | server | `feat(server)` | GP-6 |
| 6 | GP-8: JWT auth middleware | server | `feat(server)` | GP-7 |
| 7 | GP-10: SvelteKit project setup | web | `feat(web)` | GP-2 |
| 8 | GP-11: Auth pages (login/register) | web | `feat(web)` | GP-7, GP-10 |
| 9 | GP-12: Dashboard page | web | `feat(web)` | GP-8, GP-11 |

### v0.3.0 — Leaderboards + Profiles
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 10 | GP-14: Multi-user game stats | core | `feat(core)` | GP-8 |
| 11 | GP-15: Leaderboard endpoint | server | `feat(server)` | GP-14 |
| 12 | GP-16: Player profile endpoint | server | `feat(server)` | GP-14 |
| 13 | GP-17: Leaderboard page | web | `feat(web)` | GP-15 |
| 14 | GP-18: Player profile page | web | `feat(web)` | GP-16 |

### v0.4.0 — Tasks + Notes
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 15 | GP-20: Task board API integration | server | `feat(server)` | GP-8 |
| 16 | GP-21: Notes API (read-only) | server | `feat(server)` | GP-8 |
| 17 | GP-22: Task board page (kanban) | web | `feat(web)` | GP-20 |
| 18 | GP-23: Markdown note viewer | web | `feat(web)` | GP-21 |

### v0.5.0 — Universe CRUD + Viewer
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 19 | GP-25: Universe CRUD endpoints | server | `feat(server)` | GP-3, GP-8 |
| 20 | GP-27: Universe viewer component | web | `feat(web)` | GP-25 |
| 21 | GP-29: Universe browser/discovery | web | `feat(web)` | GP-25 |

### v0.6.0 — Universe Editor + Themes
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 22 | GP-26: Tile map editor | web | `feat(web)` | GP-25 |
| 23 | GP-28: Theme system + picker | web | `feat(web)` | GP-27 |

### Plugins Repo v0.1.0 (separate)
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 24 | GP-31: Plugin template | universes | `feat(universes)` | GP-3 |
| 25 | GP-32: Tetris plugin | universes | `feat(universes)` | GP-31 |
| 26 | GP-33: Snake+Invaders+PointSet+Poker | universes | `feat(universes)` | GP-31 |

### E2E (continuous)
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 27 | GP-35: Server E2E smoke test | testing | `test(server)` | GP-8, GP-15, GP-25 |

### co auto (meta — builds the tool that runs this roadmap)
| Order | Task | Module | Commit Prefix | Depends On |
|-------|------|--------|---------------|------------|
| 0 | GP-37: Task selector | co-cli | `feat(cli)` | — |
| 0 | GP-38: Context builder | co-cli | `feat(cli)` | GP-37 |
| 0 | GP-39: Claude Code launcher | co-cli | `feat(cli)` | GP-38 |
| 0 | GP-40: Acceptance criteria reviewer | co-cli | `feat(cli)` | GP-39 |
| 0 | GP-41: Task status updater | co-cli | `feat(cli)` | GP-40 |
| 0 | GP-42: Auto-cycle loop | co-cli | `feat(cli)` | GP-41 |

## Port Assignments (Dedicated Test Environment)

| Service | Port | Purpose |
|---------|------|---------|
| Game Server | 8742 | Game API (stats, wallet, leaderboards) |
| Tasks Service (co-web) | 8743 | Task/notes API (projects, tasks, comments) |
| SvelteKit Dev | 5173 | Web frontend (dev server) |
| SvelteKit Preview | 4173 | Web frontend (production preview) |

## `co auto` Invocation

```bash
# Build the auto command first (GP-36 epic)
cd $CO_WORKSPACE
cargo build

# Then run the GP roadmap
co auto --space gp --cycle --stop-on-fail

# Or run a single task
co auto --space gp --task GP-2

# Dry run (show what would execute)
co auto --space gp --dry-run
```
