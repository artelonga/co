# CO — HTTP API Catalog

**Snapshot:** 2026-05-13 · workspace 2.8.1
**Source:** `co-web/src/server.rs` + sub-routers in `*_routes.rs` and `storage_dashboard.rs`.
**Cross-reference:** the interactions registry (`co-web/e2e/interactions/registry.yaml`) is exposed at **`GET /api/v1/interactions/openapi.json`** as an OpenAPI 3.1 surface. That registry is the canonical typed contract for the *content* operations (entries, vault, references). Auth/admin/quilombo/chat are NOT yet in the registry — they're documented here only.

**Auth legend:**

| Tag | Meaning |
|---|---|
| **anon** | no auth required (anonymous-clone routes still create a universe under the hood) |
| **authed** | `crate::auth::require_auth` middleware — JWT cookie or `Authorization: Bearer` |
| **token-or-jwt** | `require_auth_with_token` — JWT cookie OR long-lived API token (vault + blob) |
| **owner** | inside `universe_content_api` — passes `universe_visibility_gate` + `universe_writer_gate` (owner/admin/member) |
| **visibility** | `universe_visibility_gate` only — readable if public or owned by caller |
| **admin** | `crate::auth::require_admin` OR GitHub-gated (`AllowedAdmins`) for `/api/v1/gestao/*` |

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
| POST | `/api/v1/auth/uat-login` | anon | CO-44 — `yuri/uat` on UAT, 404 in prod |
| POST | `/api/v1/auth/exchange-session` | authed | CO-214 — short-lived handover → 7-day ES256 JWT |
| GET | `/auth/co-handover` | authed | CO-206 — issue handover token, server-side redirect |
| POST | `/api/v1/auth/recovery/{...}` | authed | CO-165 — recovery channels CRUD + verify |
| POST | `/api/v1/auth/forgot-password` | anon | CO-165 — initiate reset |
| POST | `/api/v1/auth/reset-password` | anon | CO-165 — complete reset |
| POST | `/api/v1/auth/onboard-with-email` | anon | CO-190 — passwordless onboarding |
| POST | `/api/v1/auth/onboard-with-email/verify` | anon | CO-190 — verify code |
| POST | `/api/v1/auth/token` | authed | CO-35 — create long-lived API token |
| GET | `/api/v1/auth/tokens` | authed | List API tokens |
| DELETE | `/api/v1/auth/tokens/{id}` | authed | Revoke API token |

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

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/universes/` | visibility | List universes (public + caller's owned/member) |
| POST | `/api/v1/universes/` | authed | Create universe |
| GET | `/api/v1/universes/search` | anon | Search public universes |
| GET | `/api/v1/universes/public` | anon | Public universe directory |
| GET | `/api/v1/universes/{slug}` | visibility | Universe info |
| PUT | `/api/v1/universes/{slug}` | owner | Update name/visibility |
| DELETE | `/api/v1/universes/{slug}` | owner | Delete universe |
| GET | `/api/v1/universes/{slug}/config` | visibility | Universe config |
| PUT | `/api/v1/universes/{slug}/config` | owner | Update config |
| GET | `/api/v1/universes/{slug}/theme.css` | anon | Compiled theme CSS |
| GET | `/api/v1/universes/{slug}/projects` | visibility | List projects in universe |
| POST | `/api/v1/universes/{slug}/clone` | anon | Auto-clone template (anonymous flow) |
| POST | `/api/v1/universes/{slug}/duplicate` | authed | Duplicate as new universe |
| POST | `/api/v1/universes/{slug}/claim` | authed | Claim anon clone after login |
| GET | `/api/v1/universes/{slug}/members` | visibility | List members |
| POST | `/api/v1/universes/{key}/members` | owner | Add member |
| DELETE | `/api/v1/universes/{key}/members/{user_id}` | owner | Remove member |
| GET | `/api/v1/universes/{slug}/subscription` | authed | My subscription state |
| GET | `/api/v1/universes/{slug}/subscribers` | visibility | List subscribers |
| POST | `/api/v1/universes/{slug}/jobs/doc-gen` | owner | Submit doc-generation job |
| POST | `/api/v1/universes/apply-template-all` | admin | Apply template across all universes |
| GET | `/api/v1/universes/quilomboaraucaria/stats` | anon | Special-cased stats (CO-41) |
| POST | `/api/v1/universes/{slug}/invitations` | authed | Create invitation (CO-188) |
| GET | `/api/v1/me/universes` | authed | Bucketed: owned/member/subscribed (CO-191) |
| GET | `/api/v1/themes` | anon | List available themes per tier |

---

## entries — `/api/v1/universes/{slug}/...` (entry_routes.rs, all under owner/visibility gates)

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/{slug}/manifest` | visibility | Universe manifest (all entries) |
| GET | `/{slug}/query` | visibility | Query DSL (CO-74) |
| GET | `/{slug}/entries` | visibility | List entries (filter by type) |
| POST | `/{slug}/entries` | owner | Create entry |
| GET | `/{slug}/entries/tags` | visibility | Tag counts |
| GET | `/{slug}/entries/tree` | visibility | Hierarchical tree |
| GET | `/{slug}/entries/similar` | visibility | CO-164 — semantic similar |
| GET | `/{slug}/entries/history` | visibility | Entry change history |
| GET | `/{slug}/citations` | visibility | Inbound references |
| GET | `/{slug}/relations/inbound` | visibility | CO-153 inbound typed FKs |
| GET | `/{slug}/relations/outbound` | visibility | CO-153 outbound typed FKs |
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
| GET/PUT/DELETE | `/{slug}/vault/{*path}` | token-or-jwt | File CRUD (typical Obsidian REST shape) |

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
| GET | `/api/v1/push/vapid-key` | anon | CO-201 — VAPID public key |
| POST | `/api/v1/me/push-subscriptions` | authed | Subscribe to push |
| GET | `/api/v1/me/push-subscriptions` | authed | List subscriptions |
| DELETE | `/api/v1/me/push-subscriptions/{id}` | authed | Unsubscribe |

---

## interactions — `/api/v1/interactions/*` (interactions.rs — REGISTRY-DRIVEN)

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
| GET/POST/PUT/DELETE | `/{u}/references/works/...` | owner | CRUD reference cards |

---

## admin — `/api/v1/admin/*`, `/api/v1/gestao/*` (admin_routes.rs + gestao_routes.rs + dev_board.rs)

`/api/v1/admin/*` = JWT + email-gate (admin tier).
`/api/v1/gestao/*` = GitHub OAuth gate (`AllowedAdmins` from env `GESTAO_GITHUB_ADMINS`).

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/admin/dashboard` | admin | Cross-system dashboard |
| GET | `/api/v1/admin/users/origin-breakdown` | admin | User signup origin breakdown |
| GET | `/api/v1/admin/co-dev` | admin | CO-43 dev board info |
| GET | `/api/v1/admin/co-dev/entries` | admin | Dev board entries |
| GET | `/api/v1/admin/co-dev/entries/tags` | admin | Dev board tags |
| GET | `/api/v1/admin/leads` | admin | CO-183 leads queue |
| PATCH | `/api/v1/admin/leads/{id}` | admin | Update lead |
| GET | `/api/v1/admin/telemetry/summary` | admin | Telemetry summary (CO-46) |
| GET | `/api/v1/admin/telemetry/export` | admin | CSV export |
| GET | `/api/v1/admin/telemetry/crud-summary` | admin | CRUD events summary |
| GET | `/api/v1/admin/storage` | admin | Admin storage dashboard |
| POST | `/api/v1/ab/flags` | admin | CO-121 — A/B flag CRUD |
| GET | `/api/v1/ab/flags` | admin | List flags |
| PUT | `/api/v1/ab/flags/{key}` | admin | Toggle flag |
| POST | `/api/v1/gestao/webhooks` | admin (gh) | CO-168 register outbound webhook |
| GET | `/api/v1/gestao/webhooks` | admin (gh) | List webhooks |
| PUT | `/api/v1/gestao/webhooks/{id}` | admin (gh) | Update |
| DELETE | `/api/v1/gestao/webhooks/{id}` | admin (gh) | Delete |
| GET | `/api/v1/gestao/webhooks/{id}/deliveries` | admin (gh) | Delivery log |
| POST | `/api/v1/uat/changes` | admin | CO-45 UAT change promotion |
| GET | `/api/v1/uat/export-patch` | admin | Export changes as patch |
| POST | `/v1/log-drains/vercel/{universe_id}` | shared-secret | CO-124 Vercel log drain receiver |

---

## storage — storage_dashboard.rs

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/v1/admin/storage` | admin | Cluster-wide storage breakdown |
| GET | `/api/v1/me/storage` | authed | Per-user storage (CO-128) |
| GET | `/api/v1/universes/{slug}/storage` | owner | Per-universe breakdown |
| GET | `/api/v1/cache/stats` | anon | LRU cache stats (CO-79) |

---

## analytics + telemetry — public ingestion

| Method | Path | Auth | Purpose |
|---|---|---|---|
| POST | `/api/v1/telemetry/event` | anon | Single event |
| POST | `/api/v1/telemetry/events` | anon | Batch (Plausible-style) |
| GET | `/api/v1/analytics/public/summary` | anon | CO-179 public summary |
| GET | `/api/v1/analytics/public/recent` | anon | Recent activity |
| GET | `/api/v1/analytics/public/popularity` | anon | CO-180 popularity |

---

## quilombo — `/api/v1/quilombo/*` (quilombo_routes.rs, 1152 LoC)

CO-41 hosted-tenant: an entire parallel community CMS on top of CO's primitives. Separate auth (`/quilombo/auth/login`), separate content types (`publicacoes`, `eventos`, `missoes`, `membros`), separate admin (`/quilombo/admin/*`). 30+ routes — see file for exhaustive list. Documented here only as "exists, parallel surface."

---

## board (legacy) — `/api/projects/*`

The original kanban API, predates universes. Still wired; new code should prefer `/api/v1/universes/.../entries`.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/projects/{key}` | anon | Project info |
| GET | `/api/projects/{key}/tasks` | anon | List tasks |
| POST | `/api/projects/{key}/tasks` | authed | Create task |
| PUT/DELETE | `/api/projects/{key}/tasks/{id}` | authed | Update/delete |
| POST | `/api/projects/{key}/tasks/bulk-update` | authed | Bulk update |
| POST | `/api/projects/{key}/tasks/bulk-delete` | authed | Bulk delete |
| GET | `/api/projects/{key}/tasks/{id}/comments` | anon | List comments |
| POST | `/api/projects/{key}/tasks/{id}/comments` | authed | Create comment |
| GET | `/api/projects/{key}/activity` | anon | Activity feed |
| GET | `/api/projects/{key}/dashboard` | anon | Dashboard |

---

## game — `/api/v1/games/*`, `/api/v1/players/*` (game_routes.rs, 923 LoC)

CO-38 yggdrasil — game plugins. See `game_routes.rs` for full list (~12 endpoints): health, plugins, register/legacy-login, leaderboards (per-game + global), recent activity, player profile, profile, wallet, record-result, stats.

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

---

## Health

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/api/health` | anon | Liveness |
| GET | `/api/health/deep` | anon | Includes storage check |
