---
type: doc
title: Delivery Pipeline
---

# Delivery Pipeline

CO-398 introduced a **delivery pipeline** for project universes: task status is driven by version-control and deploy events instead of manual drag-and-drop. The board becomes a live map of the work's actual state.

## The invariant — review on localhost, approve, merge

However the statuses are spelled, the pipeline is always the same three gestures, in this order:

1. **Review on localhost.** The change is seen *running* — `co serve`, a Quartz preview, or an optional manual staging preview (`co-artelonga-staging`) — not just read as a diff. This is why a `review` card without a `preview_url` is signalled as incomplete.
2. **Approve.** The stakeholder says yes to what they saw. Approval is a human gate: automation fills statuses, it never grants approval.
3. **Merge.** The merge is the durable record of that approval — and the only thing that may trigger a production deploy. Prod follows merge, never precedes it.

CO speaks two version-control vocabularies — traditional git (code universes, PRs on GitHub) and CO-native versioning backed by jujutsu (content universes; states, branches, proposals and merges as universe operations, not a git wrapper). The three gestures map onto both:

| Gesture | Traditional git (code) | CO-native / jujutsu (content) |
|---|---|---|
| Open work | branch `feat/CO-<n>-…` | `jj new` — the working copy is itself a change, snapshotted automatically |
| Iterate | conventional commits, pushed | `jj commit -m "tipo(escopo): …"` — states on the universe |
| **Review on localhost** | PR opened (`CO-<n>` in title) + reviewer runs it (`co serve` / optional staging preview), `preview_url` attached | proposal (proposta) open + stakeholder sees it served locally (`co serve` / `co construir` preview) |
| **Approve** | PR approval, given after seeing the preview | stakeholder approves the proposta (visitors go through suggest/review) |
| **Merge ⇒ deploy** | PR merged = the approval record; prod-direct deploy follows | mesclar / `jj bookmark set main -r @-` + `jj git push`; then `co push` or `construir` publishes |

### Example — a code change, the git way

```bash
git checkout -b feat/CO-412-novo-filtro     # card → started
# … TDD commits …                            # card → in_progress
gh pr create --title "feat: novo filtro (CO-412)"   # card → review
co serve                                     # reviewer sees it RUN on localhost (or optional staging preview)
# approval happens here, looking at the running thing
scripts/safe-merge-pr.sh artelonga/co <pr>   # card → done ⇒ deploy
```

### Example — a content change, the jj way

```bash
cd ~/projects/miguel
# edit content/diario/2026-06-18.md          (jj snapshots the working copy)
jj commit -m "feat(diario): nota de reuniao"
co serve                                     # stakeholder reviews on localhost
# approval happens here
jj bookmark set main -r @-                   # the "merge": main now points at the approved state
jj git push                                  # the approval record leaves the machine
co push --remote https://co.artelonga.com.br --token $CO_TOKEN   # deploy
```

Same shape both times: nothing reaches production that was not first seen running on localhost and approved, and the merge — PR merge or bookmark move — is the artifact that proves it.

## Status sequence

| Status | Meaning | Triggered by |
|---|---|---|
| `todo` | Backlog / prioritized | Board creation / manual drag |
| `started` | Branch opened | Git branch named `CO-<n>-…` created |
| `in_progress` | First commit pushed | `push` event on the branch |
| `review` | PR open, preview attached | PR opened referencing `CO-<n>` |
| `done` | Merged to main | PR merged |

Legacy boards using `[todo, doing, done]` continue to work — the pipeline enum applies only to universes created after CO-398.

## GitHub webhook setup

1. Configure `CO_GITHUB_WEBHOOK_SECRET` in the CO server environment:
   ```bash
   flyctl secrets set CO_GITHUB_WEBHOOK_SECRET=$(openssl rand -hex 20) -a co-artelonga
   ```

2. In your GitHub repository settings → Webhooks, add:
   - **Payload URL**: `https://co.artelonga.com.br/api/v1/delivery/github?universe=<your-universe-slug>`
   - **Content type**: `application/json`
   - **Secret**: the value from step 1
   - **Events**: select *Branch or tag creation*, *Pushes*, and *Pull requests*

3. CO will automatically advance task statuses on:
   - Branch `feat/CO-<n>-…` created → task `n` moves to `started`
   - First push on the branch → task moves to `in_progress`
   - PR opened with `CO-<n>` in title or body → task moves to `review`, PR URL attached
   - PR merged → task moves to `done`, `deploy.triggered` event emitted

## Manual transitions

Manual drag-and-drop always works. Automation fills the status when a git event fires, but never locks a card. If you drag a card forward before the PR is opened, the next GitHub event will not overwrite a status that is already ahead.

## Review card: two legs

A task in `review` displays two pieces of information:
- **PR link** (`pr_url` frontmatter field) — set automatically from the pull request URL
- **Preview link** (`preview_url` frontmatter field) — set manually or by a CI job

If `preview_url` is absent, the board signals the review is incomplete. Add it via the entry editor or the vault API:

```bash
# Attach a preview URL to a task in review
curl -X PUT https://co.artelonga.com.br/api/v1/universes/<slug>/entries/<path> \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' \
  -d '{"frontmatter": {"preview_url": "http://localhost:5173"}}'
```

## Lead-time metrics

Every status transition is recorded in the `task_status_log` table. Query the dashboard endpoint for per-column averages:

```bash
curl -s https://co.artelonga.com.br/api/v1/universes/<slug>/delivery/metrics | jq .
```

Response example:

```json
{
  "universe_key": "co",
  "done_count": 12,
  "lead_time_per_status": [
    { "status": "in_progress", "avg_seconds": 86400, "count": 12 },
    { "status": "review",      "avg_seconds": 28800, "count": 10 },
    { "status": "started",     "avg_seconds": 3600,  "count": 12 }
  ]
}
```

## EDA events

| Event | When | Payload |
|---|---|---|
| `task.status_changed` | Any status transition (manual or automated) | `{path, title, from, to, trigger}` |
| `deploy.triggered` | When a task reaches `done` | `{entry_path, universe_key, trigger}` |

Subscribe to `deploy.triggered` to hook CO-395 `construir` (content universes) or trigger a prod-direct deploy for code universes.
