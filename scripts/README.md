# scripts/

Operational scripts for the CO project. Listed by category.

---

## Merge & Release

| Script | Purpose |
|--------|---------|
| `safe-merge-pr.sh <repo> <pr-number>` | Squash-merge a PR (polls until mergeable), then auto-archives the task and prunes its worktree |
| `release-commit.sh <version> [theme]` | Consolidate CHANGELOG-PENDING notes into a release commit + version bump |
| `ship-task.sh <TASK-ID> [--draft]` | Rebase a co-auto worktree on main, push, and open a PR |

---

## Task Archive (CO-301)

| Script | Purpose |
|--------|---------|
| `archive-task.sh <TASK-ID>` | Create/overwrite `docs/task-archive/<TASK-ID>.json` for a merged task |
| `prune-worktrees.sh [--apply]` | Audit + remove merged/archived worktrees (dry-run by default) |
| `co-task <subcommand>` | Query the task archive |
| `backfill-task-archives.sh [--limit N] [--commit]` | Retroactively archive the last N merged PRs |

### Lifecycle

Every `safe-merge-pr.sh` run now automatically:

1. Pulls `main` (gets the squash-merge commit)
2. Runs `archive-task.sh <TASK-ID>` → writes `docs/task-archive/<TASK-ID>.json`
3. Commits + pushes the archive file on `main`
4. Runs `prune-worktrees.sh --apply` → removes the just-merged worktree

### `archive-task.sh`

```
scripts/archive-task.sh CO-279
```

Reads spec frontmatter, `gh pr list`, `git log`, and `CHANGELOG-PENDING/<TASK-ID>.md`.
Writes `docs/task-archive/<TASK-ID>.json`. Idempotent (overwrites on re-run).

### `prune-worktrees.sh`

```
scripts/prune-worktrees.sh          # dry-run
scripts/prune-worktrees.sh --apply  # actually prune
```

A worktree is prunable only when **all** are true:

- Branch is merged (appears in GitHub merged PR list)
- `docs/task-archive/<TASK-ID>.json` exists
- No uncommitted changes
- No commits ahead of `origin/main`

Locked worktrees (agent worktrees) are always skipped.

### `co-task`

```
co-task list [--since YYYY-MM-DD] [--label LABEL] [--type TYPE] [--module MODULE]
co-task show    CO-279    # full JSON
co-task summary CO-279    # human-readable one-pager
co-task diff    CO-279    # git show <merge_sha> --stat
co-task open    CO-279    # open PR URL in browser
```

### `backfill-task-archives.sh`

```
scripts/backfill-task-archives.sh --limit 50 --commit
```

Skips tasks that already have an archive. Creates a single commit
`chore(archive): backfill CO-N..CO-M task archives`.

---

## Backup & Restore

| Script | Purpose |
|--------|---------|
| `backup.sh` | Snapshot the prod database |
| `backup-prod.sh` | Full prod backup to Fly volume |
| `backup-to-disk.sh` | Backup to local disk |
| `restore.sh` | Restore from snapshot |
| `restore-from-disk.sh` | Restore from local disk backup |

## Deployment & Operations

| Script | Purpose |
|--------|---------|
| `smoke-prod.sh` | Smoke-test production |
| `operationalize-prod.sh` | One-time prod environment setup |
| `setup-all-universes.sh` | Seed all universes |
| `seed-prod-universes.sh` | Seed production universes |
| `sync-all.sh` | Sync all content sources |
| `fly-snapshots.sh` | Manage Fly.io volume snapshots |

## Content & Data

| Script | Purpose |
|--------|---------|
| `bulk-upload.py` | Bulk-upload markdown files to CO via Vault API |
| `co-query.py` | Query CO content via API |
| `co-export.py` | Export content from CO |
| `duplicate-universe.sh` | Duplicate a universe |
| `copy-to-user.sh` | Copy template content to a user universe |
