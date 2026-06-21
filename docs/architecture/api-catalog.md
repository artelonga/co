# CO — HTTP API Catalog

**Snapshot:** 2026-06-05 · workspace 2.40.0
**Source:** `co-web/src/server.rs` + sub-routers in `*_routes.rs` and `storage_dashboard.rs`.
**Cross-reference:** the interactions registry (`co-web/e2e/interactions/registry.yaml`) is exposed at **`GET /api/v1/interactions/openapi.json`** as an OpenAPI 3.1 surface. That registry is the canonical typed contract for the *content* operations (entries, vault, references). Auth/admin/quilombo/chat are NOT yet in the registry — they're documented here only.

## How to add a route

1. Implement the handler and register it in the appropriate `*_routes.rs` file.
2. Add a row to the table in the relevant section below.
3. Run `cd co-web && npm run openapi:gen` to regenerate `co-web/openapi.yaml`.

**Auth legend:**

| Tag | Meaning |
|---|---|
| **anon** | no auth required (anonymous-clone routes still create a universe under the hood) |
| **authed** | `crate::auth::require_auth` middleware — JWT cookie or `Authorization: Bearer` |
| **token-or-jwt** | `require_auth_with_token` — JWT cookie OR long-lived API token (vault + blob) |
| **owner** | inside `universe_content_api` — passes `universe_visibility_gate` + `universe_writer_gate` (owner/admin/member) |
| **visibility** | `universe_visibility_gate` only — readable if public or owned by caller |
| **admin** | `crate::auth::require_admin` OR GitHub-gated (`AllowedAdmins`) for `/api/v1/gestao/*` |
| **telemetry:read** | `Scoped<TelemetryRead>` (CO-448) — JWT/session OR an API token carrying the `telemetry:read` capability; a token without it → 403 |

## Response envelope + version headers (CO-456 / CO-278-A)

Every `/api/v1/*` response carries two additive headers, **always** (with or
without opt-in):

| Header | Value | Meaning |
|---|---|---|
| `X-API-Version` | `1.0` | Public API contract version |
| `X-Co-Server-Version` | workspace version (e.g. `3.12.0`) | Running CO release |

**Opt-in envelope.** Send `X-API-Envelope: 1` (or
`Accept: application/vnd.co.v1+json`) and a single response layer wraps the JSON
body in the unified shape:

```jsonc
// success (HTTP < 400)
{ "data": <original body>, "meta": { "request_id": "<uuid>", "api_version": "1.0" }, "errors": null }

// error (HTTP >= 400)
{ "data": null, "meta": { "request_id": "<uuid>", "api_version": "1.0" }, "errors": [ { "code": "...", "message": "...", "field": "?", "hint": "?" } ] }
```

`meta` propagates `page`/`page_size`/`total` (best-effort) when the handler
already exposes them. **Without the opt-in marker the body is byte-identical to
the legacy shape** — so the SPA and existing consumers are unaffected. Non-JSON
responses (HTML, CSS, streams, blobs) are never wrapped. Flipping the envelope
to the default is a later one-liner once consumers adopt the header.

---

## auth — `/api/v1/auth/*` (auth_routes.rs + auth.rs)

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/auth/login` | anon | Request email magic-code (CO-188) |
| POST | `/api/v1/auth/verify` | anon | Exchange code → session cookie |
| GET | `/api/v1/auth/me` | authed | Current user + tier |
| GET | `/api/v1/auth/stats` | authed | Per-user storage stats |
| POST | `/api/v1/auth/logout` | anon | Clear session cookie |
| POST | `/api/v1/auth/password-login` | anon | CO-85 — username+password (any env) |
| POST | `/api/v1/auth/signup` | anon | CO-175 — usuario + password (+ optional email), rate-limited 100/day |
| GET | `/api/v1/auth/google/status` | anon | Whether Google OAuth is configured |
| GET | `/api/v1/auth/google/start` | anon | Begin Google OAuth (oauth_google.rs) |
| GET | `/api/v1/auth/google/callback` | anon | Google OAuth callback |
| GET | `/api/v1/auth/github/status` | anon | Whether GitHub OAuth is configured |
| GET | `/api/v1/auth/github/start` | anon | CO-415 — begin GitHub OAuth (oauth_github.rs) |
| GET | `/api/v1/auth/github/callback` | anon | CO-415 — GitHub OAuth callback |
| POST | `/api/v1/auth/uat-login` | anon | CO-44 — `yuri/uat` on UAT, 404 in prod |
| GET | `/auth/co-handover` | authed | CO-206 — issue handover token, server-side redirect |
| POST | `/api/v1/auth/forgot-password` | anon | CO-165 — initiate reset |
| POST | `/api/v1/auth/reset-password` | anon | CO-165 — complete reset |
| POST | `/api/v1/auth/onboard-with-email` | anon | CO-190 — passwordless onboarding |
| POST | `/api/v1/auth/onboard-with-email/verify` | anon | CO-190 — verify code |
| POST | `/api/v1/auth/token` | authed | CO-35 — create long-lived API token; CO-448 accepts optional `scopes` (capability/bundle list) for a least-privilege token |
| GET | `/api/v1/auth/tokens` | authed | List API tokens |
| DELETE | `/api/v1/auth/tokens/{id}` | authed | Revoke API token |
| GET | `/api/v1/auth/login-options` | anon | CO-303 — login methods available |

**Recovery channels (CO-165) — all authed, mounted at /api/v1/auth/recovery/**

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/auth/recovery/channels` | authed | List recovery channels |
| POST | `/api/v1/auth/recovery/channels` | authed | Add recovery channel |
| POST | `/api/v1/auth/recovery/channels/verify` | authed | Verify channel code |
| DELETE | `/api/v1/auth/recovery/channels/{id}` | authed | Remove recovery channel |
| POST | `/api/v1/auth/forgot-password/verify` | anon | Verify reset token |
| POST | `/api/v1/auth/change-password` | authed | Change password (authed) |

**OIDC (provider role) — `/oauth/*` + well-known (oidc_routes.rs)**

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/.well-known/openid-configuration` | anon | OIDC discovery |
| GET | `/.well-known/jwks.json` | anon | JWKS (ES256 public key) |
| GET | `/oauth/authorize` | authed | OAuth2 authorization code flow |
| POST | `/oauth/token` | anon | Token exchange |
| GET | `/oauth/userinfo` | token | OIDC userinfo |
| POST | `/api/v1/gestao/oauth/clients` | admin | Register OAuth client |
| GET | `/api/v1/gestao/oauth/clients` | admin | List clients |

---

## universes — `/api/v1/universes/*` (universe_routes.rs + invitation_routes.rs)

Note: `GET /api/v1/universes` (list) and `POST /api/v1/universes` (create) resolve
from `.route("/", ...)` nested at `/api/v1/universes` — verified manually.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/universes` | visibility | List universes (public + caller's owned/member) |
| POST | `/api/v1/universes` | authed | Create universe (CO-444: API-token or session; body may set `visibility` ∈ private/public/unlisted and `parent_key`) |
| GET | `/api/v1/universes/search` | anon | Search public universes |
| GET | `/api/v1/universes/public` | anon | Public universe directory |
| GET | `/api/v1/universes/trash` | authed | List caller's trashed + archived universes (CO-96) |
| GET | `/api/v1/universes/{slug}` | visibility | Universe info |
| PUT | `/api/v1/universes/{slug}` | owner | Update name/visibility/parent_key (CO-423; CO-444 visibility ∈ private/public/unlisted, API-token accepted) |
| DELETE | `/api/v1/universes/{slug}` | owner | Soft-delete universe — sets `deleted_at`, recoverable via trash (CO-96) |
| POST | `/api/v1/universes/{slug}/archive` | owner | Archive (soft-hide) universe (CO-96) |
| POST | `/api/v1/universes/{slug}/restore` | owner | Restore a trashed/archived universe (CO-96) |
| GET | `/api/v1/universes/{slug}/config` | visibility | Universe config |
| PUT | `/api/v1/universes/{slug}/config` | owner | Update config |
| PATCH | `/api/v1/universes/{slug}/source` | owner | Patch universe source config |
| GET | `/api/v1/universes/{slug}/theme.css` | anon | Compiled theme CSS |
| GET | `/api/v1/universes/{slug}/projects` | visibility | List projects in universe |
| POST | `/api/v1/universes/{slug}/clone` | anon | Auto-clone template (anonymous flow) |
| POST | `/api/v1/universes/{slug}/duplicate` | authed | Duplicate as new universe |
| POST | `/api/v1/universes/{slug}/claim` | authed | Claim anon clone after login |
| POST | `/api/v1/universes/{slug}/apply-template` | owner | Apply template universe |
| POST | `/api/v1/universes/{slug}/reindex` | owner | Rebuild search index |
| GET | `/api/v1/universes/{slug}/members` | visibility | List members |
| POST | `/api/v1/universes/{key}/members` | owner | Add member |
| DELETE | `/api/v1/universes/{key}/members/{user_id}` | owner | Remove member |
| GET | `/api/v1/universes/{slug}/subscription` | authed | My subscription state |
| POST | `/api/v1/universes/{slug}/subscribe` | authed | Subscribe to universe |
| DELETE | `/api/v1/universes/{slug}/subscribe` | authed | Unsubscribe |
| PUT | `/api/v1/universes/{slug}/subscribe/pin` | authed | Pin subscription |
| DELETE | `/api/v1/universes/{slug}/subscribe/pin` | authed | Unpin subscription |
| GET | `/api/v1/universes/{slug}/subscribers` | visibility | List subscribers |
| POST | `/api/v1/universes/{slug}/jobs/doc-gen` | owner | Submit doc-generation job |
| GET | `/api/v1/universes/{slug}/jobs/doc-gen/last-error` | owner | Last doc-gen job error |
| GET | `/api/v1/universes/{slug}/invitations` | owner | List universe invitations |
| POST | `/api/v1/universes/apply-template-all` | admin | Apply template across all universes |
| GET | `/api/v1/universes/quilomboaraucaria/stats` | anon | Special-cased stats (CO-41) |
| POST | `/api/v1/universes/{slug}/invitations` | authed | Create invitation (CO-188) |
| POST | `/api/v1/universes/{slug}/invites` | owner | Create invite — `{email\|handle, role}` (CO-444; API-token accepted) |
| GET | `/api/v1/universes/{slug}/invites` | owner | List pending invites (CO-444) |
| DELETE | `/api/v1/universes/{slug}/invites/{token}` | owner | Revoke an invite by token_hash (CO-444) |
| POST | `/api/v1/universes/{slug}/invites/{token}/accept` | authed | Accept an invite (universe-scoped, CO-444) |
| POST | `/api/v1/universes/{slug}/messages` | anon | Contact form — message the universe owner; body `{from_email,from_name,subject,body,website?}` (`website` = honeypot); rate-limited 5/hr/IP; delivers via email + in-app notification (CO-326) |
| GET | `/api/v1/me/universes` | authed | Bucketed: owned/member/subscribed (CO-191) |
| GET | `/api/v1/themes/available` | anon | List available themes per tier |
| GET | `/api/v1/themes/{preset}` | anon | Compiled CSS for a specific theme preset |

---

## git-sync — `/api/v1/universes/{slug}/*` (gitsync/routes.rs — CO-89)

Git-backed universes: opt-in via the `git_source` column. Ingesting `git log`
produces `commit` + `profile` content entries with per-profile analytics, plus
server-side Mermaid (gantt of in-flight tasks, timeline of recent commits).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/universes/{slug}/git-source` | owner | Set/clear `git_source` + `git_branch` (CO-89) |
| POST | `/api/v1/universes/{slug}/git-sync` | owner | Ingest `git log` now → commit/profile entries + analytics (CO-89) |
| POST | `/api/v1/universes/{slug}/recompute-analytics` | owner | Recompute every profile's analytics struct (CO-89) |
| GET | `/api/v1/universes/{slug}/gantt` | authed | Mermaid gantt of in-flight tasks, swimlane by assignee (CO-89) |
| GET | `/api/v1/universes/{slug}/commit-timeline` | authed | Mermaid timeline of recent commits, grouped by day (CO-89) |

---

## graph-views — `/api/v1/graph-views/*` (graph_view_routes.rs)

CO-345: publishable saved graph views (universe + type filter + depth + root + layout seed).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/graph-views` | anon | List public graph views |
| POST | `/api/v1/graph-views` | authed | Create a saved graph view |
| GET | `/api/v1/graph-views/{slug}` | anon-if-public, authed-if-private | Fetch a saved view |
| PATCH | `/api/v1/graph-views/{slug}` | authed (owner) | Update visibility/filters |
| DELETE | `/api/v1/graph-views/{slug}` | authed (owner) | Delete |

---

## workspace — `/api/v1/universes/{key}/workspaces/*` + `/u/{universe}/sala` (workspace_routes.rs)

CO-352: Sala primitive — spatial canvas anchored to a universe, with per-user state and shareable public state.
CO-399: scope expansion — `/sala` (every visible universe) and `/sala?u=a,b` (subset) render the same surface; state keys on `workspace_states.scope` (`*` or a normalized `a,b`), nodes/edges use `key::path` cross-universe notation.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/universes/{key}/workspaces/{slug}/state` | authed (member) | Get current user's workspace state for this sala |
| GET | `/api/v1/universes/{key}/workspaces/{slug}/state/public` | anon | Get public/shared workspace state |
| PUT | `/api/v1/universes/{key}/workspaces/{slug}/state` | authed | Persist workspace state (positions, custom edges) |
| POST | `/api/v1/universes/{key}/workspaces/{slug}/state/share` | authed | Publish workspace state as shareable |
| GET | `/api/v1/workspace-states/{token}` | anon | Fetch a shared workspace state by token (carries `scope` for multi-universe shares) |
| GET | `/api/v1/sala/scope` | anon | CO-399: resolve a scope (`?u=a,b`, or all if absent) → visibility-filtered universes |
| GET | `/api/v1/sala/state` | anon | CO-399: caller's saved state for a multi-universe scope (`?scope=a,b`) |
| PUT | `/api/v1/sala/state` | authed | CO-399: persist state for a multi-universe scope (`?scope=a,b`) |
| POST | `/api/v1/sala/state/share` | authed | CO-399: publish a multi-universe scope state as shareable |
| GET | `/u/{universe}/sala` | anon | Sala SPA shell (default workspace for universe) |
| GET | `/u/{universe}/sala/{workspace_slug}` | anon | Sala SPA shell (specific workspace) |
| GET | `/sala` | anon | CO-399: Sala SPA shell — every caller-visible universe (`/sala?u=a,b` for a subset) |

## workspace-templates — `/api/v1/universes/{slug}/workspace-templates/*` (workspace_template_routes.rs)

CO-355: per-universe `_workspace.yaml` template registry — seeds Sala layouts.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/universes/{slug}/workspace-templates` | visibility | List available workspace templates |
| GET | `/api/v1/universes/{slug}/workspace-templates/{template_slug}` | visibility | Fetch a specific template |
| POST | `/api/v1/universes/{slug}/workspaces/{workspace_slug}/from-template/{template_slug}` | authed | Seed a new workspace from a template |

---

## calendar — `/api/v1/universes/{slug}/calendar` (time/routes.rs)

CO-387: per-universe calendar lens configuration (`_calendar.yaml`) for the
`<co-time-grid>` time-rendering primitive. Lens metadata only — no entry data.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/universes/{slug}/calendar` | anon | Calendar lens config: `{ default_lens, lenses[] }`. Each lens: `id`, `name`, `scale` (linear or log), `epoch_ms?`, `canonical_field?` (default `event_at_ms`), `canonical_type?` (i64_ms, f64_years or i64_units), `week_length_days?`, `weekday_names?[]`, `month_length?`, `timezone?`, `display_format?`, `display_unit?`, `epoch?`, `custom_event_field?`, `cell_duration_ms?`, `break_duration_ms?`, `label_periods?[{name,at}]`. Falls back to the built-in Gregorian lens when the universe has no `_calendar.yaml`. |

---

## scrum (universe) — `/api/v1/universes/{key}/scrum/*` (scrum/routes.rs)

CO-368: per-universe Scrum artifacts. PBIs and Sprints are ordinary CO entries
(`entry_type = "pbi" | "sprint"`); the cadence comes from the universe's
`_scrum.yaml`. A universe without a manifest reports `enabled: false` and is
otherwise unchanged. Distinct from the global CO-372 calendar at `/api/v1/scrum/*`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/universes/{key}/scrum/manifest` | anon | Parsed `_scrum.yaml` (`enabled`, `sprint_length_days`, `sprint_start_anchor`, `release_tag_pattern`, `roles`, `default_dod`, …) plus the computed `current_sprint` (or `null` when disabled / no anchor). |
| GET | `/api/v1/universes/{key}/scrum/sprints` | anon | `{ sprints[], total }` — all `sprint` entries, sorted by `number`. Each: `{ path, type, title, frontmatter, body }`. |
| GET | `/api/v1/universes/{key}/scrum/sprints/current` | anon | Current sprint computed from cadence + anchor: `{ number, start_at, end_at, release_window }`. `404` when Scrum is disabled or `sprint_start_anchor` is missing. |
| GET | `/api/v1/universes/{key}/scrum/backlog` | anon | `{ pbis[], total }` — `pbi` entries. Query `status=` (backlog/ready/in-sprint/done) and `sprint=` filter. |
| POST | `/api/v1/universes/{key}/scrum/pbi` | anon | Create a PBI (sugar over an entry write at `scrum/pbi/{id}.md`). Body: `{ id, title, priority, points?, status?, acceptance?[], sprint?, assignees?[], epic?, body? }`. `422` on invalid frontmatter. Returns the created entry. |
| PATCH | `/api/v1/universes/{key}/scrum/pbi/{id}/dod` | anon | Check off a Definition-of-Done item. Body: `{ index, done }`. Seeds the checklist from `_scrum.yaml.default_dod` on first toggle. Returns `{ path, dod[] }`. `404` if the PBI is absent. |

---

## suggest/review — `/api/v1/universes/{slug}/suggest` + `/{slug}/review*` (review_routes.rs)

CO-354: entry lifecycle (draft → reviewed → published) with anon submissions. Outside the writer gate so anonymous visitors can suggest; review actions enforce ownership in-handler.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/universes/{slug}/suggest` | visibility | Submit a suggestion (anon-friendly, rate-limited) — creates a draft entry |
| GET | `/api/v1/universes/{slug}/review` | owner (in-handler) | List draft + reviewed entries for the review queue |
| PATCH | `/api/v1/universes/{slug}/review` | owner (in-handler) | Edit a draft before approving |
| POST | `/api/v1/universes/{slug}/review/approve` | owner (in-handler) | Approve → review_status published |
| POST | `/api/v1/universes/{slug}/review/reject` | owner (in-handler) | Reject → entry removed |
| POST | `/api/v1/universes/{slug}/publish` | owner (in-handler) | CO-418: render-review-publish with traceback — conventional commit + semver intent, stamps `requested_by`, preserves `source`; idempotent re-publish; surfaces `origin`/`requested_by` typed relations |

---

## entries — `/api/v1/universes/{slug}/...` (entry_routes.rs, all under owner/visibility gates)

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/{slug}/manifest` | visibility | Universe manifest (all entries) |
| GET | `/{slug}/query` | visibility | Query DSL (CO-74) |
| POST | `/{slug}/query` | authed | CO-244 SQL query (read-only, auth required) |
| GET | `/{slug}/entries` | visibility | List entries (filter by type) |
| POST | `/{slug}/entries` | owner | Create entry |
| GET | `/{slug}/entries/tags` | visibility | Tag counts |
| GET | `/{slug}/entries/tree` | visibility | Hierarchical tree |
| GET | `/{slug}/entries/similar` | visibility | CO-164 — semantic similar |
| GET | `/{slug}/entries/history` | visibility | Entry change history |
| GET | `/{slug}/entries/versions` | visibility | CO-54 — pre-overwrite version snapshots (audit trail) |
| GET | `/{slug}/entries/diff` | visibility | CO-75 — op-level diff of an entry between two instants (`?path=&from=&to=` RFC3339) |
| GET | `/{slug}/changelog` | visibility | CO-75 — auto-generated Keep-a-Changelog from the op log (`?since=&until=` RFC3339) |
| GET | `/{slug}/entries/popular` | visibility | Popular entries by view count |
| GET | `/{slug}/entries/{*path}` | visibility | Get entry by path (CO-75: `?as_of=<RFC3339>` reconstructs a past state) |
| PUT | `/{slug}/entries/{*path}` | owner | Update entry |
| DELETE | `/{slug}/entries/{*path}` | owner | Delete entry |
| POST | `/{slug}/translate/{*path}` | owner | CO-416 — generate pt↔en content twin (`?to=en`); 503 if backend unconfigured |
| GET | `/{slug}/translation/{*path}` | visibility | CO-416 — does a translation twin exist (+ stale flag) |
| GET | `/{slug}/citations` | visibility | Inbound references |
| GET | `/{slug}/citations/orphan-wikilinks` | visibility | Wikilinks with no target entry |
| GET | `/{slug}/relations/inbound` | visibility | CO-153 inbound typed FKs |
| GET | `/{slug}/relations/outbound` | visibility | CO-153 outbound typed FKs |
| GET | `/{slug}/graph` | visibility | CO-335 — entry relation graph (nodes + edges) |
| GET | `/{slug}/dev-tasks` | visibility | CO-272 dev board tasks for dogfooding |
| GET | `/{slug}/timeline` | visibility | CO-396 project-timeline lens. Query `group_by=epic\|module\|status` (default `status`). Returns `{ group_by, lanes[{key,label,items[{id,title,entry_type,span{start_ms,end_ms}}]}], markers[{id,title,kind,at_ms}], backlog[{id,title,entry_type}] }` — the same entries as the kanban board laid out by the shared `game_core::time_layout` engine (lanes + release/milestone markers + a no-date backlog). |
| POST | `/{slug}/states` | owner | CO-native versioning Phase 1 — snapshot |
| GET | `/{slug}/states/diff` | visibility | Diff between two states |
| POST | `/{slug}/branches` | owner | Phase 2 — named pointer to state |
| PUT | `/{slug}/branches/{name}` | owner | Advance branch |
| POST | `/{slug}/proposals` | owner | Phase 3 — state-snapshot proposal |
| POST | `/{slug}/merges` | owner | Merge proposal |
| POST | `/{slug}/proposals/inline` | authed | Lightweight inline proposal (non-owner) |
| POST | `/{slug}/proposals/decide` | owner | Merge/reject inline proposal |
| GET | `/api/v1/me/inbound-proposals` | authed | Inbox of inbound proposals across owned universes |
| GET | `/api/v1/search` | authed | CO-164 cross-universe semantic search |

**Op-log (CO-95) — versioning primitives under universe_content_api**

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/{slug}/ops` | visibility | List operations in op-log |
| GET | `/{slug}/oplog` | visibility | Full op-log with payloads |
| GET | `/{slug}/replay` | visibility | Replay op-log to a point |
| GET | `/{slug}/diff` | visibility | Diff between op-log positions |
| GET | `/{slug}/op-diff` | visibility | Per-op diff view |
| POST | `/{slug}/promote` | owner | Promote branch tip |
| POST | `/{slug}/revert` | owner | Revert to prior state |
| POST | `/{slug}/cherry-pick` | owner | Cherry-pick operation |

---

## vault — `/api/v1/universes/{slug}/vault/*` (vault_routes.rs)

Obsidian-compat surface — accepts JWT cookie OR long-lived API token.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/{slug}/vault/` | token-or-jwt | List notes/files |
| GET | `/{slug}/vault/tags` | token-or-jwt | Tag counts |
| GET | `/{slug}/vault/tree` | token-or-jwt | File tree |
| POST | `/{slug}/vault/search` | token-or-jwt | Full-text search |
| POST | `/{slug}/vault/clip` | token-or-jwt | Web clipper ingest |
| GET/PUT/PATCH/DELETE | `/{slug}/vault/{*path}` | token-or-jwt | File CRUD (Obsidian REST shape; PATCH for partial updates) |
| POST | `/{slug}/vault/{*path}` | token-or-jwt | Create file at path |

---

## chat — `/api/v1/universes/{slug}/chat/*` (chat_routes.rs + chat_ws.rs)

All authed; room visibility further enforced inside handlers via `resolve_role()`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/{slug}/chat/rooms` | authed | List rooms in universe |
| POST | `/{slug}/chat/rooms` | authed | Create room (member-only) |
| GET | `/{slug}/chat/rooms/{room_slug}/members` | authed | Room member list |
| GET | `/{slug}/chat/rooms/{room_slug}/messages` | authed | Paginated history |
| POST | `/{slug}/chat/rooms/{room_slug}/messages` | authed | Post message |
| PATCH | `/{slug}/chat/rooms/{room_slug}/messages/{msg_id}` | authed | Edit own message |
| DELETE | `/{slug}/chat/rooms/{room_slug}/messages/{msg_id}` | authed | Delete own message |
| WS | `/api/v1/universes/{slug}/chat/rooms/{room_slug}/ws` | authed (in-handler) | Live fan-out + presence |

---

## dm — `/api/v1/*` (dm_routes.rs)

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/dms/with/{user_id}` | authed | Open or get DM room |
| GET | `/api/v1/me/dms` | authed | DM inbox |
| POST | `/api/v1/dms/{room_id}/read` | authed | Mark read |
| POST | `/api/v1/dms/{room_id}/mute` | authed | Toggle mute |
| PUT | `/api/v1/me/dm-policy` | authed | Who can DM me |
| POST | `/api/v1/users/{user_id}/block` | authed | Block user |
| DELETE | `/api/v1/users/{user_id}/block` | authed | Unblock |

---

## notifications — `/api/v1/me/notifications` (notification_routes.rs)

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/me/notifications` | authed | List notifications |
| POST | `/api/v1/me/notifications/{id}/read` | authed | Mark single read |
| POST | `/api/v1/me/notifications/read-all` | authed | Mark all read |
| GET | `/api/v1/me/notification-preferences` | authed | Get prefs |
| PUT | `/api/v1/me/notification-preferences` | authed | Update prefs |
| GET | `/api/v1/notifications/vapid-public-key` | anon | CO-201 — VAPID public key |
| POST | `/api/v1/me/push-subscriptions` | authed | Subscribe to push |
| GET | `/api/v1/me/push-subscriptions` | authed | List subscriptions |
| DELETE | `/api/v1/me/push-subscriptions/{id}` | authed | Unsubscribe |

---

## interactions — `/api/v1/interactions/*` (interactions.rs — REGISTRY-DRIVEN)

Note: `GET /api/v1/interactions` (root listing) resolves from `.route("/", ...)` nested at
`/api/v1/interactions` — verified manually.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/interactions/openapi.json` | anon | **OpenAPI 3.1 generated from `e2e/interactions/registry.yaml`** |
| GET | `/api/v1/interactions` | anon | Discover registered interactions |
| GET | `/api/v1/interactions/{id}` | anon | Inspect one |
| POST | `/api/v1/interactions/{id}` | (501 reserved) | Execution not yet wired |

**This is the canonical typed surface** — any new content endpoint should be added to the registry first, then the OpenAPI surface updates automatically. Today the registry covers the entries/vault/references operations only.

---

## proposals — see entries section above

Proposals live under `entry_routes` group conceptually but are wired in `proposal_routes.rs`. Both state-snapshot proposals (owner-gated, Phase 3) and inline lightweight proposals (authed non-owner) are listed in the entries table.

---

## vault — see vault section above

---

## assets — `/api/v1/universes/{slug}/assets/*` (asset_routes.rs + blob_routes.rs)

All inside `universe_content_api` gate (owner for write). `blob/*` is read-through with visibility gate. Body limit raised to 50 MB on this router.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/{slug}/assets` | visibility | List assets in universe |
| POST | `/{slug}/assets` | owner | Upload asset (drag-and-drop) |
| GET | `/{slug}/assets/tags` | visibility | Tag counts |
| GET | `/{slug}/assets/{sha256}` | visibility | Asset metadata |
| DELETE | `/{slug}/assets/{sha256}` | owner | Delete |
| POST | `/{slug}/assets/{sha256}/tags` | owner | Add tags |
| DELETE | `/{slug}/assets/{sha256}/tags/{tag}` | owner | Remove tag |
| GET | `/{slug}/blob/{*path}` | visibility | Serve raw blob (PDF viewer fallback) |
| GET | `/api/v1/blobs/{hash}` | token-or-jwt | CAS read (CO-163 BaseBackend shim) |
| HEAD | `/api/v1/blobs/{hash}` | token-or-jwt | CAS exists check |
| POST | `/api/v1/blobs` | token-or-jwt | CAS write |

---

## references — `/api/v1/universes/{u}/references/*` (reference_routes.rs)

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/{u}/references/orphan-blobs` | visibility | Blobs with no reference card |
| GET | `/{u}/references/broken-cards` | visibility | Cards with missing blob/file |
| GET | `/{u}/references/works` | visibility | List works |
| GET | `/{u}/references` | visibility | List reference cards |
| POST | `/{u}/references` | owner | Create reference card |
| GET | `/{u}/references/{*path}` | visibility | Get reference card |
| PUT | `/{u}/references/{*path}` | owner | Update reference card |
| DELETE | `/{u}/references/{*path}` | owner | Delete reference card |

---

## admin — `/api/v1/admin/*` (admin_routes.rs + dev_board.rs)

`/api/v1/admin/*` = JWT + email-gate (admin tier).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/admin/dashboard` | admin | Cross-system dashboard |
| GET | `/api/v1/admin/users/origin-breakdown` | admin | User signup origin breakdown |
| GET | `/api/v1/admin/chat/origin-breakdown` | admin | CO-204 — chat message origin-universe breakdown; CO-448 requires capability `chat:read` |
| GET | `/api/v1/admin/co-dev` | admin | CO-43 dev board info |
| GET | `/api/v1/admin/co-dev/entries` | admin | Dev board entries |
| GET | `/api/v1/admin/co-dev/entries/tags` | admin | Dev board tags |
| GET | `/api/v1/admin/co-dev/entries/{*path}` | admin | Get dev board entry by path |
| PUT | `/api/v1/admin/co-dev/entries/{*path}` | admin | Update dev board entry |
| POST | `/api/v1/admin/changelog/reindex` | admin | Re-index changelog from git |
| GET | `/api/v1/admin/leads` | admin | CO-183 leads queue |
| PATCH | `/api/v1/admin/leads/{id}` | admin | Update lead |
| GET | `/api/v1/admin/telemetry/summary` | admin | Telemetry summary (CO-46) |
| GET | `/api/v1/admin/telemetry/export` | admin | CSV export |
| GET | `/api/v1/admin/telemetry/crud-summary` | admin | CRUD events summary |
| GET | `/api/v1/admin/pipeline/summary` | admin | CO-88 content-pipeline per-universe stats |
| GET | `/api/v1/admin/workers/status` | admin | Worker/job status |
| GET | `/api/v1/admin/deployments` | admin | Deployment history |
| POST | `/api/v1/admin/deployments/refresh` | admin | Refresh deployment data |
| POST | `/api/v1/ab/flags` | admin | CO-121 — A/B flag CRUD |
| GET | `/api/v1/ab/flags` | admin | List flags |
| PUT | `/api/v1/ab/flags/{key}` | admin | Toggle flag |
| GET | `/api/v1/uat/changes` | admin | CO-45 UAT change list |
| POST | `/api/v1/uat/export-patch` | admin | Export changes as patch |
| POST | `/v1/log-drains/vercel/{universe_id}` | shared-secret | CO-124 Vercel log drain receiver |
| POST | `/api/v1/admin/backup/snapshot` | admin | CO-365 — force snapshot via active backup backend |
| GET | `/api/v1/admin/backup/snapshots` | admin | CO-365 — list snapshots from backup backend |
| POST | `/api/v1/admin/backup` | admin | CO-459 — reconstructable local snapshot (VACUUM INTO + sha256 manifest); returns verified manifest |
| POST | `/api/v1/admin/sweep` | admin | CO-459 — junk sweep; dry-run default, `?apply=true` to delete |

---

## gestao — `/api/v1/gestao/content` (gestao_routes.rs — GitHub OAuth gate)

Content management via GitHub PAT. Routes under `/api/v1/gestao/` gated by `GESTAO_GITHUB_ADMINS`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/gestao/schema-status` | admin (gh) | CO-361 — current schema version + app version |
| GET | `/api/v1/gestao/universos/indisponiveis` | admin (gh) | CO-406 — list universes the pool marks unavailable |
| POST | `/api/v1/gestao/universos/{key}/reabrir` | admin (gh) | CO-406 — reopen a degraded universe (no restart) |
| GET | `/api/v1/gestao/analytics/resumo` | admin (gh) | CO-378 — analytics resumo with private-path visibility |
| GET | `/gestao` | admin (gh) | CO-361 — gestao SPA shell page |
| POST | `/api/v1/gestao/webhooks` | admin (gh) | CO-168 register outbound webhook |
| GET | `/api/v1/gestao/webhooks` | admin (gh) | List webhooks |
| PUT | `/api/v1/gestao/webhooks/{id}` | admin (gh) | Update |
| DELETE | `/api/v1/gestao/webhooks/{id}` | admin (gh) | Delete |
| GET | `/api/v1/gestao/webhooks/{id}/deliveries` | admin (gh) | Delivery log |
| POST | `/api/v1/gestao/validar` | admin (gh) | Validate markdown frontmatter |
| POST | `/api/v1/gestao/publicar` | admin (gh) | Publish markdown file to universe |
| GET | `/api/v1/gestao/relatos` | admin (gh) | List all relatos |
| POST | `/api/v1/gestao/relatos` | admin (gh) | Create relato |
| PUT | `/api/v1/gestao/relatos/{id}` | admin (gh) | Update relato |
| DELETE | `/api/v1/gestao/relatos/{id}` | admin (gh) | Delete relato |
| GET | `/api/v1/gestao/eventos` | admin (gh) | List all eventos |
| POST | `/api/v1/gestao/eventos` | admin (gh) | Create evento |
| PUT | `/api/v1/gestao/eventos/{id}` | admin (gh) | Update evento |
| DELETE | `/api/v1/gestao/eventos/{id}` | admin (gh) | Delete evento |
| GET | `/api/v1/gestao/membros` | admin (gh) | List all membros |
| POST | `/api/v1/gestao/membros` | admin (gh) | Create membro |
| PUT | `/api/v1/gestao/membros/{id}` | admin (gh) | Update membro |
| DELETE | `/api/v1/gestao/membros/{id}` | admin (gh) | Delete membro |
| GET | `/api/v1/gestao/quadro` | admin (gh) | List board items |
| POST | `/api/v1/gestao/quadro` | admin (gh) | Create board item |
| PUT | `/api/v1/gestao/quadro/{id}` | admin (gh) | Update board item |
| DELETE | `/api/v1/gestao/quadro/{id}` | admin (gh) | Delete board item |
| GET | `/api/v1/gestao/manifesto` | admin (gh) | Universe Iceberg manifest |
| POST | `/api/v1/gestao/manifesto/reconstruir` | admin (gh) | Rebuild manifest from source |

---

## billing — `/api/v1/me/billing/*` + webhook (billing/routes.rs)

CO-366 — conversion + payment wiring (register → paid). Provider chosen via
`CO_BILLING_PROVIDER` (default `manual`; `hostinger` for the live integration)
behind the `BillingProvider` trait. The `mark-paid` admin route uses the same
email-admin auth as `/gestao/resumo` (CO-360).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/me/billing/checkout` | authed | Start checkout for `{ plan }`, returns `{ url, session_id, expires_at, provider }` |
| GET | `/api/v1/me/billing/status` | authed | Current `{ tier, plan, paid_at, billing_provider }` |
| POST | `/api/v1/billing/webhook/{provider}` | anon (HMAC) | Provider webhook — verifies signature, flips tier on success |
| POST | `/api/v1/gestao/users/{id}/mark-paid` | admin (email) | Manual provider — mark a user paid |

---

## storage — storage_dashboard.rs

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/admin/storage` | admin | Cluster-wide storage breakdown |
| GET | `/api/v1/me/storage` | authed | Per-user storage (CO-128) |
| GET | `/api/v1/universes/{slug}/storage` | owner | Per-universe breakdown |
| GET | `/api/v1/cache/stats` | anon | LRU cache stats (CO-79) |

---

## invitations — `/api/v1/invitations/*` (invitation_routes.rs)

CO-188 single-use invite tokens. Preview is public; accept requires auth.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/invitations/{token}` | anon | Preview invitation before accepting |
| POST | `/api/v1/invitations/{token}/accept` | authed | Accept invite, join universe |
| GET | `/api/v1/me/invitations` | authed | My pending invitations |
| POST | `/api/v1/me/invitations/accept` | authed | Accept invite via me/ (token in body) |

---

## leads — `/api/v1/leads` (lead_routes.rs)

CO-183 lead capture form (marketing).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/leads` | anon | Submit interest (public) |

---

## openapi — `/api/openapi.json` + `/api/docs` (openapi_routes.rs)

Machine-readable spec and interactive explorer.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/openapi.json` | anon | OpenAPI 3.1 JSON spec |
| GET | `/api/docs` | anon | Interactive API explorer |

---

## agent-sessions — `/api/v1/agent/sessions` (agent_session_routes.rs)

CO-275 agent session tracking for the co-auto pipeline.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/agent/sessions` | anon | List agent sessions (public kanban) |
| GET | `/api/v1/agent/sessions/latest` | anon | Latest session summary |
| POST | `/api/v1/agent/sessions` | token-or-jwt | Record new agent session |

---

## usage — `/api/v1/usage/*` (usage_routes.rs)

CO-426 fleet usage ingestion + active-launcher registry + Gestão dashboard
queries. All require a vault token or JWT (`require_auth_with_token`); the read
endpoints additionally require `tier == "admin"`. The `sessions` producer is
CO-425; `summary` also backs the CO-427 downshift policy. CO-437 enriches the
producer payload (tool usage, output size, PR link) and adds the
model×universe cross-tab plus a per-session listing.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/v1/metrics` | bearer (OTLP) | OTLP/HTTP metrics receiver — ingests Claude Code native `claude_code.*` telemetry into `usage_sessions` (CO-457) |
| POST | `/api/v1/usage/sessions` | token-or-jwt | Ingest a session's token/cost + metadata report (CO-425/CO-437 producer) |
| GET | `/api/v1/usage/sessions` | admin | Recent per-session listing with tool usage + PR link (CO-437) |
| POST | `/api/v1/usage/heartbeat` | token-or-jwt | Register/refresh an active launcher (in-memory TTL) |
| GET | `/api/v1/usage/summary` | admin | Aggregates by universe\|model\|machine\|task; `?cross=universe,model` adds the cross-tab matrix (CO-437) |
| GET | `/api/v1/usage/active` | admin | Launchers active right now |
| GET | `/api/v1/usage/projects` | admin | Fleet roll-up: usage + board + last deploy per universe |

---

## telemetry — `/api/v1/telemetry/*` (telemetry_api.rs)

CO-453 (CO-278-E) public, read-only telemetry/analytics surface. Every route is
gated by `Scoped<TelemetryRead>` (CO-448 `telemetry:read`): a least-privilege
API token carrying the capability reads telemetry without being admin-pleno, and
a full JWT/session also passes. Reuses existing tables/aggregations only — no new
schema (`agent_sessions` CO-275, `deployment_snapshots` CO-273, `usage_sessions`
CO-426, `release_notes` CO-334). Nested under `/api/v1/telemetry/*` to avoid the
pre-existing anon, task-scoped `/api/v1/agent/sessions` kanban reads.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/telemetry/agent/sessions` | telemetry:read | List agent sessions; filters `?model=&since=`, pagination `?limit=&offset=` |
| GET | `/api/v1/telemetry/agent/sessions/{id}` | telemetry:read | One agent session by id (404 if absent) |
| GET | `/api/v1/telemetry/deployments` | telemetry:read | Latest deployment snapshot per app/unit; `?limit=` caps units |
| GET | `/api/v1/telemetry/metrics/throughput` | telemetry:read | Token/session throughput over a window (`?since=`), with per-model breakdown |
| GET | `/api/v1/telemetry/metrics/token-budget` | telemetry:read | Spend vs `CO_AUTO_SOFT_LIMIT_5H_TOKENS` budget over a window (`?since=`) |
| GET | `/api/v1/telemetry/metrics/release-cadence` | telemetry:read | Releases per period from `release_notes` (`?since=` ISO date, `?limit=`) |

---

## ai — `/api/v1/ai/*` (ai_routes.rs)

CO-328 AI provider endpoints (Ollama / Claude Code hook).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/ai/query` | authed | AI completion query |
| GET | `/api/v1/ai/status` | authed | AI provider status |

---

## chat-llm — `/api/v1/chat/*` (chat_routes.rs)

CO-332 Public LLM chat + deployment status. No auth required (rate-limited).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/chat/{slug}` | anon | LLM chat on a universe |
| GET | `/api/v1/deployments/status` | anon | Latest deployment status |

---

## changelog — `/api/v1/changelog` (changelog_routes.rs)

CO-260 cross-version changelog viewer.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/changelog` | anon | Changelog entries |
| GET | `/api/v1/changelog/feed` | anon | Changelog Atom/RSS feed |
| GET | `/api/v1/changelog/repos` | anon | Repositories in changelog |
| GET | `/changelog/embed` | anon | Iframeable releases-as-sprints view (CSP frame-ancestors *.artelonga.com.br) |

---

## resolve — `/api/v1/resolve` (resolve_routes.rs)

CO-338 surface-key resolution: turn a logical `<key>::<path>` cross-universe
reference into a live deployment URL by walking the universe registry (`key`,
`parent_key`, `surface_dns`) to the nearest deployable ancestor.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/resolve` | anon | Resolve a surface ref (query `?ref=<key>::<path>`) → `{ url, universe, deployable_ancestor }`; unknown key → 404, ambiguous/malformed → 400 |

---

## events — `/api/v1/events` (event_bus / live_routes.rs)

CO-380 universal event bus + CO-381 live timeline WebSocket fanout.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/events` | authed (cookie or token) | WebSocket — subscribe to scoped event stream; filter via `?scope=mine|universe:<key>|public|admin` |
| GET | `/api/v1/events/bridge` | bridge-jwt | CO-384 — federated bridge: remote producers (Yggdrasil) dial in to publish events; CO is the hub |

### CO-382 CI/CD event types

Published by CI workflows to the event bus via the bridge endpoint.
Rendered in the `/agora` live timeline with 🛠️ icon.

| `event_type` | Payload fields | When published |
|---|---|---|
| `ci.step.passed` | `step` (1-10), `step_name`, `pr_number`, `co_id`, `duration_ms` | Each CI step completes successfully |
| `ci.step.failed` | `step`, `step_name`, `pr_number`, `co_id`, `error` | Each CI step fails |
| `ci.dod.verified` | `co_id`, `dod_pct`, `passed`, `total`, `pr_number` | DoD verification passes (100%) |
| `ci.dod.failed` | `co_id`, `dod_pct`, `passed`, `total`, `failed_items`, `pr_number` | DoD verification fails |
| `release.gate.passed` | `version`, `wave_co_ids`, `sprint_number` | Thursday release gate clears |
| `release.gate.blocked` | `blocked_co_ids`, `sprint_number`, `reason` | Thursday release gate is blocked |

All CI events use `visibility: System` (admin stream only, never forwarded to anonymous WebSocket).

---

## sync — `/api/v1/sync/*` + `/api/v1/me/sync/*` (sync/routes.rs)

CO-385 CRUD action tree — Mac-style UPSERT conflict resolution for cross-device sync.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/me/sync/conflicts` | authed | List the caller's outstanding sync conflicts (returns `SyncConflict[]`) |
| POST | `/api/v1/sync/conflicts/{id}/resolve` | authed | Resolve a conflict; body `{action, apply_to_all_matching?}` — actions: `keep_both`, `ignore`, `replace`, `update`, `upsert`, `accept_delete`, `keep_local` |

---

## feedback — `/api/v1/feedback/*` (feedback_routes.rs)

CO-333 feedback system. Submissions are public; management is owner-only.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/feedback` | anon | Submit feedback (universe-wide) |
| GET | `/api/v1/feedback/all/public` | anon | Public feedback aggregated across universes |
| GET | `/api/v1/feedback/{key}` | owner | List feedback for universe |
| PATCH | `/api/v1/feedback/{key}` | owner | Update feedback status |
| GET | `/api/v1/feedback/{universe_key}/public` | anon | Public feedback mural for universe |
| GET | `/api/v1/feedback/{universe_key}/item/{id}` | anon | Single feedback item |
| GET | `/api/v1/feedback/{universe_key}/entry/{*entry_path}` | anon | Feedback for a specific entry |
| POST | `/api/v1/feedback/{universe_key}/{*entry_path}` | anon | Submit feedback for entry |

---

## analytics + telemetry — public ingestion

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/telemetry/event` | anon | Single event |
| POST | `/api/v1/telemetry/events` | anon | Batch (Plausible-style) |
| GET | `/api/v1/analytics/public/summary` | anon | CO-179 public summary |
| GET | `/api/v1/analytics/public/recent` | anon | Recent activity |
| GET | `/api/v1/analytics/public/popularity` | anon | CO-180 popularity |
| POST | `/api/v1/analytics/public/rollups` | authed | Ingest per-universe rollup (CO-340) |
| GET | `/api/v1/analytics/public/funnel` | anon | CO-378 — conversion funnel report with private-path redaction |

---

## quilombo — `/api/v1/quilombo/*` (quilombo_routes.rs, 1152 LoC)

CO-41 hosted-tenant: parallel community CMS. Separate auth, content types, admin.

**Public routes**

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/quilombo/publicacoes` | anon | List published relatos |
| GET | `/api/v1/quilombo/publicacoes/{slug}` | anon | Get relato by slug |
| GET | `/api/v1/quilombo/paginas/{slug}` | anon | Get static page |
| GET | `/api/v1/quilombo/eventos` | anon | List eventos |
| GET | `/api/v1/quilombo/eventos/{id}` | anon | Get evento by ID |
| GET | `/api/v1/quilombo/eventos/slug/{slug}` | anon | Get evento by slug |
| GET | `/api/v1/quilombo/missoes` | anon | List missoes |
| GET | `/api/v1/quilombo/missoes/{id}` | anon | Get missao with participants |
| GET | `/api/v1/quilombo/membros` | anon | List membros |
| GET | `/api/v1/quilombo/membros/{usuario}` | anon | Member public profile |
| GET | `/api/v1/quilombo/comentarios` | anon | List comments |
| POST | `/api/v1/quilombo/comentarios` | anon | Post comment (anonymous allowed) |
| POST | `/api/v1/quilombo/contato` | anon | Contact form |
| GET | `/api/v1/quilombo/tags` | anon | All tags |
| GET | `/api/v1/quilombo/tags/{tag}` | anon | Relatos by tag |
| GET | `/api/v1/quilombo/upload/{filename}` | anon | Serve upload file |
| GET | `/api/v1/quilombo/fotos/{filename}` | anon | Serve photo file |

**Autenticado (JWT quilombo)**

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/quilombo/auth/login` | anon | Quilombo login (user+password) |
| POST | `/api/v1/quilombo/auth/cadastro` | anon | Quilombo registration |
| POST | `/api/v1/quilombo/auth/link-co-account` | authed | Link to CO account |
| GET | `/api/v1/quilombo/perfil` | authed | My quilombo profile |
| PUT | `/api/v1/quilombo/perfil` | authed | Update profile |
| GET | `/api/v1/quilombo/mensagens` | authed | My messages |
| POST | `/api/v1/quilombo/mensagens` | authed | Send message |
| POST | `/api/v1/quilombo/missoes/criar` | authed | Create missao (admin) |
| POST | `/api/v1/quilombo/missoes/{id}/participar` | authed | Join missao |
| PUT | `/api/v1/quilombo/missoes/{id}/participacoes/{uid}` | authed | Approve/reject participation |
| POST | `/api/v1/quilombo/eventos/criar` | authed | Create evento (admin) |
| PUT | `/api/v1/quilombo/eventos/{id}/editar` | authed | Update evento (admin) |
| POST | `/api/v1/quilombo/eventos/{id}/excluir` | authed | Delete evento (admin) |

**Admin quilombo**

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/quilombo/admin/telemetria` | admin | Telemetria quilombo |
| GET | `/api/v1/quilombo/admin/resumo` | admin | Admin summary |
| GET | `/api/v1/quilombo/admin/usuarios` | admin | List users |
| PUT | `/api/v1/quilombo/admin/usuarios/{id}` | admin | Update user |
| GET | `/api/v1/quilombo/admin/atividades` | admin | Activity log |

**Processos (CO-329 web-edit workflow)**

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/quilombo/alterar-pagina-na-web/runs` | authed | List web-edit runs |
| POST | `/api/v1/quilombo/alterar-pagina-na-web/preview` | authed | Preview change |
| POST | `/api/v1/quilombo/alterar-pagina-na-web/approve/{run_id}` | authed | Approve run |
| POST | `/api/v1/quilombo/alterar-pagina-na-web/revert` | authed | Revert run |

---

## board (legacy) — `/api/projects/*`

The original kanban API, predates universes. Still wired; new code should prefer `/api/v1/universes/.../entries`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/projects/{key}` | anon | Project info |
| GET | `/api/projects/{key}/tasks` | anon | List tasks |
| GET | `/api/projects/{key}/tasks/{id}` | anon | Get single task |
| POST | `/api/projects/{key}/tasks` | authed | Create task |
| PUT/DELETE | `/api/projects/{key}/tasks/{id}` | authed | Update/delete |
| POST | `/api/projects/{key}/tasks/bulk-update` | authed | Bulk update |
| POST | `/api/projects/{key}/tasks/bulk-delete` | authed | Bulk delete |
| GET | `/api/projects/{key}/tasks/{id}/comments` | anon | List comments |
| POST | `/api/projects/{key}/tasks/{id}/comments` | authed | Create comment |
| GET | `/api/projects/{key}/activity` | anon | Activity feed |
| GET | `/api/projects/{key}/dashboard` | anon | Dashboard |
| POST | `/api/projects` | authed | Create project |
| DELETE | `/api/projects/{key}` | authed | Delete project |

---

## experiment — `/api/experiment/*` (inline in router.rs)

A/B experiment assignment — used by the SPA to choose variants.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/experiment/variant` | anon | Get current variant |
| POST | `/api/experiment/variant` | anon | Switch variant |
| POST | `/api/experiment/feedback` | anon | Record feedback |
| GET | `/api/experiment/summary` | anon | Variant summary |

---

## game — `/api/v1/*` (game_routes.rs, 923 LoC)

CO-38 yggdrasil — game plugins.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/health` | anon | Game service health |
| GET | `/api/v1/plugins` | anon | List plugins |
| POST | `/api/v1/auth/register` | anon | Register game player |
| POST | `/api/v1/auth/legacy-login` | anon | Legacy game login |
| GET | `/api/v1/games/{game_name}/leaderboard` | anon | Game leaderboard |
| GET | `/api/v1/games/leaderboard/global` | anon | Global leaderboard |
| GET | `/api/v1/games/recent` | anon | Recent game activity |
| GET | `/api/v1/players/{username}` | anon | Player public profile |
| GET | `/api/v1/profile` | authed | My game profile |
| GET | `/api/v1/wallet` | authed | My wallet |
| POST | `/api/v1/games/{game_name}/result` | authed | Record game result |
| GET | `/api/v1/games/{game_name}/stats` | authed | My game stats |

**Plugins (dynamic — loaded at runtime from `plugins/` directory)**

| Method | Path | Auth | Purpose |
|---|---|---|---|
<!-- CO-436 removed the plugin route registration (the loader's router was
     discarded — build_router(None)); this catalog row is dropped to match. -->

---

## live timeline — `/agora` (pt-BR) + `/live` (en) (live_routes.rs / server/router.rs)

CO-381: real-time deployed-interface visualization. SPA shell connecting to the CO-380 event bus WebSocket.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/agora` | anon | CO-381 — live timeline SPA (pt-BR) |
| GET | `/live` | anon | CO-381 — live timeline SPA (en) — same SPA as /agora |

---

## docs pages — seguranca / dependencias / licensa (docs_routes.rs)

CO-210: server-rendered docs viewer over the markdown in `docs/`. Literal page
routes merged before the `/{slug}` catch-all; shared collapsible/dropdown nav;
markdown → CSP-safe HTML; per-render telemetry (`page_view` / `404_route`
server-side, `page_render` from `docs-viewer.js`) with reader opt-out via
`localStorage["co_viewer_telemetry"]`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | /seguranca | anon | Security overview (docs/security/SECURITY.md) |
| GET | /seguranca/dependencias | anon | Dependencies (docs/security/dependencies.md) |
| GET | /seguranca/dependencias/decisoes | anon | Dependency decisions |
| GET | /seguranca/red-team | anon | Red-team scenarios |
| GET | /seguranca/red-team/playbook | anon | Incident playbook |
| GET | /seguranca/vapid | anon | VAPID push security |
| GET | /licensa | anon | License (AGPL v3) |
| GET | /renderers | anon | Markdown renderers guide |
| GET | /e2e-walkthrough | anon | E2E walkthrough |
| GET | /dependencias | anon | Redirect → /seguranca/dependencias |
| GET | /seguranca/{*rest} | anon | 404 page + `404_route` telemetry |

---

## kb — `/api/v1/kb/*` (kb_routes.rs)

CO-367: universal content → KB sync warehouse (history + latest + FTS).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/kb/ingest` | authed | Ingest entry version into the KB warehouse |
| GET | `/api/v1/kb/search` | authed | Full-text search over body previews (FTS5) |
| GET | `/api/v1/kb/recent` | authed | Latest synced entry versions |

---

## security — `/api/v1/gestao/security/*` (security/routes.rs)

CO-388: Glasswing security audit pipeline — scan findings + release blocker.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/gestao/security/findings` | admin (gh) | List vulnerability findings |
| GET | `/api/v1/gestao/security/findings/{id}` | admin (gh) | Finding detail |
| PATCH | `/api/v1/gestao/security/findings/{id}` | admin (gh) | Resolve finding (patched / accepted-risk / false-positive / wont-fix) |
| GET | `/api/v1/gestao/security/scan/status` | admin (gh) | Last scan status |
| POST | `/api/v1/gestao/security/scan` | admin (gh) | Trigger a scan |

## delivery — pipeline no quadro (delivery_routes.rs)

CO-398: VC/deploy events drive board status via the CO-380 bus + GitHub webhooks.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/universes/{slug}/delivery/github` | webhook secret | GitHub webhook — branch/PR/merge events → status transitions |
| GET | `/api/v1/universes/{slug}/delivery/metrics` | visibility | Lead-time metrics per column (task_status_log) |

---

## suggest/review pages — top-level, served before the SPA wildcard

CO-354 page shells (the API lives in the suggest/review section above).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/{slug}/suggest` | anon | Public suggest form page (shared/suggest.html) |
| GET | `/{slug}/review` | anon page, owner data | Owner review queue page (shared/review.html) |

---

## scrum — `/api/v1/scrum/*` (scrum/calendar.rs)

CO-372: Sprint calendar with Definition of Done + ICS export. Public endpoints, no auth required.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/scrum/calendar` | anon | Sprint calendar JSON (past + future sprints, DoD %, events). Query params: `past=6` (default), `future=4` (default). |
| GET | `/api/v1/scrum/calendar.ics` | anon | iCalendar export for Google/Apple Calendar subscription. Returns `text/calendar`. |
| GET | `/scrum/calendar` | anon | Sprint calendar SPA page (standalone HTML). |

---

## SPA + page routes (server.rs)

| Path | Purpose |
|---|---|
| `/` `/{slug}` `/{slug}/{*subpath}` | SPA shell (serve_co_index) |
| `/admin` | Admin page (server-side auth) |
| `/repl` | REPL shell over interactions API |
| `/storage` | Storage dashboard page |
| `/admin/leads.html` | Leads admin page |
| `/co/telemetria` | Telemetry dashboard for `co` universe |
| `/co/co-dev/pipeline` | CO-88 content-pipeline dashboard (per-universe stats) |
| `/settings/sync` | Sync settings (API token) |
| `/yggdrasil/{game}` | Yggdrasil game view (SPA) |
| `/notifications` | Notifications full page (CO-202) |
| `/recover` | Forgot-password page (validated server-side) |
| `/invitations/{token}` | Invitation accept page (SPA) |
| `/{slug}/assets` | Asset browser (CO-150) |
| `/linhadotempo` `/timeline` | Aliases → `/shared/timeline.html?u=tempo,universo,humanity` |

---

## WebSockets

| Path | Purpose |
|---|---|
| `WS /ws/doc/{slug}/{doc_id}` | CRDT document room (Yjs-style) |
| `WS /api/v1/sync/ws` | CO-151 SyncDelta protobuf sync |
| `WS /api/v1/universes/{slug}/chat/rooms/{room_slug}/ws` | CO-194 chat fan-out + presence |
| `WS /ws/sala/{universe_key}/{workspace_slug}` | CO-353 Sala realtime canvas — presence (cursors), node/edge LWW ops, suggest/publish fan-out; anon = read-only |

---

## Health

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/health` | anon | Liveness |
| GET | `/api/health/deep` | anon | Includes storage check |

---

## Metrics

CO-78: job-queue worker-pool metrics. Aggregate + per-kind only (no universe
keys), public like `/health`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/metrics` | anon | Queue depth, dead-letter count, autoscale recommendation, per-kind success rate + p99 latency |

---

## crawl — robots + sitemap (server/crawl_routes.rs)

CO-397: edge protection — crawl policy respects per-universe visibility and noindex.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/robots.txt` | anon | Crawl policy (private paths disallowed) |
| GET | `/sitemap.xml` | anon | Public universes/entries only |
