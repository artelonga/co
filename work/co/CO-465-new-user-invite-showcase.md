---
title: New-user invite + showcase — default personal universe + public-subscribable discovery
status: todo
---

# CO-465 — Seamless invite + onboarding showcase

> Goal: a new user can be invited and immediately *get it* — they (1) discover and
> subscribe to **public-subscribable** universes, and (2) land in **their own
> private personal universe** that exists by default.

## Findings (2026-06-18 audit)
- **Auth is ready**: web magic-code signup (CO-190) + CLI `co login` magic-code
  (PR #285). Any email → authenticated, no password. ✓
- **Pillar 1 — public-subscribable: EXISTS.** `subscriptions.rs::search_public_subscribable`
  (name/description search over `visibility='public-subscribable'`) + `subscribe`
  / `is_subscribed` / `list_universe_subscribers`. Discover + subscribe work.
- **Pillar 2 — default personal universe: MISSING.** Signup/verify creates the
  *user* but **no universe**. A new user lands with `owned: []` and must manually
  hit "Novo universo". That's the gap to close for "the universe they get by default".

## Scope
1. **Default personal universe on first login** (the keystone).
   - On user creation (verify handler), if the user owns no universe, auto-create
     one: `key = <handle>` (sanitised, collision-suffixed), `name = "<display> "`,
     `visibility = private`, owner = the user, + a default project + a welcome entry.
   - Idempotent + **non-fatal** (signup must still succeed if creation fails — this
     is the auth hot path; wrap in a guarded, logged best-effort).
   - Seed a starter entry (e.g. `content/bem-vindo.md`) so the universe isn't empty.
2. **Discovery showcase** (surface pillar 1).
   - A `/discover` (or onboarding step) listing public-subscribable universes
     (artelonga, yggdrasil, comunicacao, neuro, …) with a one-click Subscribe.
   - Wire `search_public_subscribable` to a public read endpoint + SPA view.
3. **Invite flow** — shareable invite link → magic-code signup → drop the new user
   into their personal universe with the discover panel alongside (the "showcase").
4. **CLI parity** — `co login` (✓ #285) → `co sync pull <their-universe>` works once
   the default universe exists.

## Acceptance
- Sign up fresh (web + CLI) → user immediately owns a private `<handle>` universe
  with a welcome entry, editable in the web + via `co sync`.
- `/discover` lists public-subscribable universes; Subscribe adds them to the sidebar.
- No password anywhere in the new-user path.
