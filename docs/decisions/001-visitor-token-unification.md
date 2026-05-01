# ADR-001 — Visitor token unification (CO-97)

**Date:** 2026-05-01  
**Status:** Accepted

## Context

Two surfaces share the same domain family:

| Surface | Cookie | Scope | HttpOnly |
|---|---|---|---|
| Co app (`co.artelonga.com.br`) | `visitante_id` | bare host | yes |
| Marketing site (`artelonga.com.br`) | `al_vid` | apex `.artelonga.com.br` | no |

After the 2026-04-29 marketing flip (analytics.js → Co telemetry endpoint), both
surfaces land events in `telemetry_events` but with different visitor tokens.
Attribution funnels (marketing → signup → in-app) split on join.

## Decision: Option A — Co adopts `al_vid`

Co's telemetry middleware now:

1. **Reads** `al_vid` first when present; falls back to `visitante_id` for visitors
   who hit Co before the marketing site.
2. **Emits** `al_vid` scoped to `.artelonga.com.br` with `SameSite=Lax; Secure`
   — **`HttpOnly` intentionally dropped** so the marketing JS can read/write it.

The `visitante_id` cookie is no longer set; existing holders migrate on their
next visit (they'll carry `visitante_id` until Co issues `al_vid`, at which
point future reads prefer `al_vid`).

## Security trade-off

Dropping `HttpOnly` exposes `al_vid` to client-side JS.

**Why this is acceptable:**
- `al_vid` is an analytics-only visitor token — it has no auth role, no session
  authority, and cannot elevate privilege.
- Worst-case of theft: an attacker can fake page-views or skew session
  attribution. **No account takeover risk.**
- The marketing site's `al_vid` was already JS-readable before this change.
- CSP on `co.artelonga.com.br` limits third-party script origins (see below).

## CSP review

Current `Content-Security-Policy` on Co:

- `script-src 'self'` — only Co's own origin; no third-party script can execute
  in the Co app context and read `al_vid`.
- `connect-src 'self' https://co.artelonga.com.br` — no exfiltration path for
  the token through fetch/XHR to an attacker-controlled origin.

No tightening required. The existing policy already prevents the primary XSS
exfiltration vector.

## Compat window

During the rollout period, some visitors will carry only `visitante_id`.
The fallback read path preserves their continuity. No double-counting occurs
because each visitor produces exactly one token per session regardless of
which cookie name it came from.

## Rejected alternatives

- **Option B** (marketing adopts `visitante_id`): same trade-off as A but
  requires touching the marketing repo and scoping `visitante_id` at apex.
  More surface, same risk.
- **Option C** (server-side `visitor_alias` table): correct but complex;
  lossy when `Referer` is missing; query performance cost outweighs benefit
  given the privacy posture already in place.
