# `skills/` — co-auto skill injection contract (CO-451)

co-auto loads markdown **skills** from `{workspace_root}/skills/*.md` and injects
them into each task's context. Skill selection is implemented by
`skill_names_for_task` in `dev/co-auto/src/auto.rs`; this file documents the
contract so any workspace (CO, AL, …) can ship the skills its agents expect.

A skill is just a markdown file: `skills/<name>.md`. It is injected as a
`## Skill: <name>` block. A name with no matching file degrades gracefully (it is
silently skipped), so a workspace only ships the skills it actually has.

## What gets injected, in order

Names are resolved into a single deduplicated list (first occurrence wins, so the
order below is stable):

1. **Core process skill — always.** `co-auto-process.md` is injected into *every*
   task when present in the workspace. It carries the canonical co-auto loop, the
   three gates, and the model fallback, so every agent inherits the same process
   regardless of its role or module.

2. **Label-derived skills.**

   | Label | Skill |
   |-------|-------|
   | `module:spa` / `module:editor` / `module:ui` | `spa-conventions` |
   | `module:deploy` / `module:infra` | `deploy-runbook` |
   | any other `module:*` | `rust-architecture` |
   | `type:orchestrate` | `orchestrate` |
   | `type:implement` / `type:feat` / `type:fix` | `implement` |
   | `type:review` | `review` |
   | `type:test` | `playwright-pattern` **and** `test` |
   | `type:release` / `type:deploy` | `release` |

   The `type:*` rows are the **role playbooks**: a task labelled with its role
   automatically inherits that role's skill.

3. **Explicit `skills:` frontmatter.** A task may list extra (or override) skills
   directly in its frontmatter. These are appended and deduplicated against the
   names already derived from labels:

   ```yaml
   ---
   id: 451
   title: "Some task"
   labels:
     - type:implement
   skills:
     - implement        # already label-derived → not duplicated
     - migration-template
   ---
   ```

## Adding a skill to a workspace

1. Drop `skills/<name>.md` into the workspace root.
2. If it's a role/module skill, it is picked up automatically via the label map
   above. Otherwise reference it from a task's `skills:` frontmatter.
3. To ship the universal process skill, add `skills/co-auto-process.md` — every
   task in that workspace then inherits it with no further wiring.

The session record (`skills_loaded`) reflects only the skills whose files exist on
disk, so it is an accurate audit of what was actually injected.
