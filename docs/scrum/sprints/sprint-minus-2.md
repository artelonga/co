# Sprint -2 (2026-05-01 → 2026-05-14)

**Sprint Goal**: (retrospective — inferred from PBIs)
**Release**: v2.6.0
**Velocity**: 71 PBIs delivered

## Delivered PBIs

### CO-209 — Conversas — unified chat surface with first-time welcome + member-visible privacy disclosure
_Merged: 2026-05-13_
_Release: v2.6.0_

- [ ] Click `💬 Conversas` for the first time → welcome pop-up appears
- [ ] Press `Enter` → pop-up dismisses, focus moves to chat composer
- [ ] Reload → pop-up does NOT appear again (localStorage persists)
- [ ] Click outside the modal → same dismissal behavior as Enter
- [ ] Click `×` → same
- [ ] Pop-up text in PT (and EN if language toggle is set)
- [ ] Single `💬 Conversas` button in sidebar (no more separate `📩 Mensagens`)
- [ ] Drawer opens with left rail showing two sections: "Universos" and
- [ ] Universos list includes every universe the user is a member of,
- [ ] CO universe's chat is named "CO-geral" in the rail
- [ ] Other universes' default room is "geral"
- [ ] Clicking a universe in the rail loads its chat
- [ ] Clicking a DM in the rail loads that DM
- [ ] DM section is collapsible
- [ ] Member rail visible at the bottom of the left side (or as a
- [ ] Shows all `chat_room_members` for the current conversation
- [ ] Presence dot ● online / ○ offline
- [ ] Click a member → opens DM with them (if `dm_policy` allows)
- [ ] Filter input appears when member count > 10
- [ ] For DMs, member rail shows the two parties
- [ ] Existing `chat_rooms` data is unchanged at the schema level
- [ ] The CO universe's auto-seeded room is renamed `'CO-geral'` (one
- [ ] New universes seed with name `'geral'`
- [ ] Send/edit/delete/WS broadcast all continue working
- [ ] CO-198 DM flow continues working

### CO-208 — Playwright e2e suite maintenance — unwind 12 days of API drift + rate-limit collisions
_Merged: 2026-05-13_
_Release: v2.6.0_

- [ ] e2e job passes in CI (all chromium-desktop tests).
- [ ] Rate-limit env-bypass works ONLY when `CO_ENV=test`; prod and
- [ ] `is_anonymous` test updated to match current response shape.
- [ ] No new flake — if a test fails, it fails for a real reason.
- [ ] CI run time for e2e is back under 15 min (currently times out

### CO-180 — Popularity endpoint pra ranking de serviços no home da artelonga
_Merged: 2026-05-13_
_Release: v2.6.0_

_(no acceptance criteria in spec)_

### CO-179 — Public analytics endpoints — /summary e /recent pra dashboard de artelonga
_Merged: 2026-05-13_
_Release: v2.6.0_

_(no acceptance criteria in spec)_

### CO-178 — Geo enrichment server-side (country + city) em telemetry_events
_Merged: 2026-05-13_
_Release: v2.6.0_

_(no acceptance criteria in spec)_

### CO-177 — Aceitar events de artelonga.com.br via CORS + populate universe_key
_Merged: 2026-05-13_
_Release: v2.6.0_

_(no acceptance criteria in spec)_

### CO-206 — Yggdrasil verifies CO JWKS — centralized SSO across all universes
_Merged: 2026-05-12_
_Release: v2.4.0_

- [ ] `is_allowed_return_to` safelist includes
- [ ] Test: `GET /auth/co-handover?return_to=https://yggdrasil-artelonga.fly.dev/auth/co-handover`
- [ ] Test: same with malformed `return_to` (e.g.
- [ ] `users.yggdrasil_user_id` column exists (idempotent migration).
- [ ] `/auth/co-handover` endpoint exists; validates ES256 via JWKS.
- [ ] JWKS cached 5 min (per spec; tests verify cache TTL).
- [ ] Expired token (>60s old) → 401.
- [ ] Invalid signature → 401.
- [ ] Valid token → local user provisioned in `usuarios`, session
- [ ] Anonymous landing UI links to CO login (no more email-code
- [ ] **Smoke test passes:** YG-22 poker multiplayer test (the one

### CO-184 — (spec not found)
_Merged: 2026-05-12_
_Release: v2.4.0_

_(no acceptance criteria in spec)_

### CO-205 — Artelonga signup backend — CORS + origin tracking for artelonga.com.br registrations
_Merged: 2026-05-12_
_Release: v2.4.0_

- [ ] CORS allows `https://artelonga.com.br` on auth endpoints with
- [ ] All three signup paths accept and persist `origin` field.
- [ ] `users.origin` populated correctly for new signups; existing
- [ ] Cookie set after signup is readable on artelonga.com.br (manual
- [ ] Admin breakdown endpoint works.
- [ ] Origin sanitizer rejects malformed values silently (stores NULL
- [ ] No regression on existing co.artelonga.com.br signup flows.
- [ ] Tests: 6+ covering CORS preflight, origin persist, origin sanitize,

### CO-203 — Switch Mutex<Storage> from std::sync to parking_lot — eliminate poison-on-panic cascade
_Merged: 2026-05-12_
_Release: v2.4.0_

- [ ] `parking_lot = "0.12"` added to `co-web/Cargo.toml`.
- [ ] `AppStateInner.storage` is `parking_lot::Mutex<Storage>`.
- [ ] All `state.storage.lock().unwrap()` / `.unwrap_or_else()` /
- [ ] `cargo build -p co-web` passes.
- [ ] `cargo clippy -p co-web -- -D warnings` clean.
- [ ] `cargo test -p co-web --lib -- --test-threads=1` shows no
- [ ] Deliberate panic test: temporarily insert `panic!("test")` in
- [ ] Smoke test on prod after deploy: log in via Google, view a

### CO-204 — Chat message origin telemetry — track which universe context each message was sent from
_Merged: 2026-05-12_
_Release: v2.4.0_

- [ ] Migration adds `origin_universe_key` nullable column + index.
- [ ] POST message with explicit `origin_universe_key` writes it (when
- [ ] POST message with origin the caller doesn't belong to → field
- [ ] POST message without origin → server defaults to room's
- [ ] GET messages returns the field.
- [ ] DM UI shows "via {universe_name}" subtext for messages whose
- [ ] Admin breakdown endpoint returns aggregated counts.
- [ ] Tests: 6+ covering origin write/read, privacy drop, default

### CO-191 — Unified GET /api/v1/me/universes shape — owned, member, invited, subscribed, discoverable
_Merged: 2026-05-12_
_Release: v2.4.0_

- [ ] `GET /api/v1/me/universes` returns 401 when unauthenticated.
- [ ] For an owner with 1 owned, 1 member, 1 invited, 0 subscribed, the
- [ ] `discoverable` bucket excludes universes the caller owns / is
- [ ] `discoverable` is capped at 50 entries.
- [ ] `counts` matches array lengths.
- [ ] Hidden universes are excluded except from `owned` (the owner can
- [ ] Tests: 6+ covering each bucket, role resolution, hidden filter,
- [ ] No regression on `GET /api/v1/universes` flat-list shape.

### CO-199 — Notification engine + preferences + 4 event types (Phase 5 slice 1)
_Merged: 2026-05-11_
_Release: v2.2.0_

- [ ] Migration creates `notifications` + `notification_preferences` tables
- [ ] Boot-time backfill inserts default preferences for every existing
- [ ] Chat message → notification row for every other room member.
- [ ] DM message → notification row for the other party only.
- [ ] Invitation create → notification row for invited user (when they
- [ ] `@usuario` mention in chat body → notification row for that user
- [ ] `GET /me/notifications` paginates and returns unread_count.
- [ ] Mark-read endpoints work; idempotent for already-read rows.
- [ ] Preferences GET/PUT work with partial-update semantics.
- [ ] Idempotency: posting the same chat message twice within 5s

### CO-202 — In-app notification center — 🔔 bell + dropdown + settings page (Phase 5 slice 4)
_Merged: 2026-05-11_
_Release: v2.2.0_

- [ ] 🔔 bell visible in header/sidebar when logged in (not for anonymous).
- [ ] Bell shows red-dot badge with unread count when > 0; hidden when 0.
- [ ] Click bell → dropdown opens with recent notifications.
- [ ] Click a notif row → marks read, navigates to URL, badge decrements.
- [ ] "Marcar todas" clears all unread; badge goes to 0.
- [ ] Receiving a chat WS event for an unread-eligible room bumps the
- [ ] Settings section renders all 4×3 toggle matrix + frequency radio +
- [ ] Toggles save optimistically via `PUT /me/notification-preferences`.
- [ ] Push subscribe button calls into CO-201's `subscribeToPush()`.
- [ ] Registered devices list shows + lets user revoke individual
- [ ] `/notifications` full-page route serves the SPA with the full
- [ ] All i18n strings PT + EN.
- [ ] Anonymous users see no bell.

### CO-201 — Web push notifications — browser Push API + service worker (Phase 5 slice 3)
_Merged: 2026-05-11_
_Release: v2.2.0_

- [ ] VAPID public key endpoint returns the configured key.
- [ ] `POST /me/push-subscriptions` upserts and returns 201.
- [ ] `DELETE /me/push-subscriptions/:id` removes the row.
- [ ] Service worker registers and survives page refresh.
- [ ] Receiving a notification while tab is foregrounded — no
- [ ] Receiving a notification while tab is backgrounded → system
- [ ] Click notification → tab focuses on the relevant URL (or new
- [ ] Multiple notifs from same thread coalesce via `tag` (no spam).
- [ ] Quiet hours respected.
- [ ] Disabled `push_*` event types not delivered.
- [ ] Dead subscriptions (410 Gone) pruned automatically.
- [ ] `delivered_push_at` populated after success.

### CO-200 — Email digest delivery — daily/hourly/instant per user prefs (Phase 5 slice 2)
_Merged: 2026-05-11_
_Release: v2.2.0_

- [ ] Background worker spawns at startup; logs `notif email worker
- [ ] User with `email_digest_freq=instant` and 1 unread notification
- [ ] Same user with the same notification already delivered → no
- [ ] User with `email_digest_freq=daily` and notifs created 30 min
- [ ] Same user 24h later (and unread) → email sent.
- [ ] User with `email_digest_freq=never` → never sent.
- [ ] User in quiet hours → skipped during the quiet window, sent
- [ ] User with `email_chat_message=0` and 5 chat-message notifs +
- [ ] After send, `notifications.delivered_email_at` populated.
- [ ] After send failure, retry next tick (up to 5 attempts).
- [ ] Email subject + body localized per `users.language`.
- [ ] CSS-inlined HTML email renders correctly in Gmail desktop +

### CO-198 — Private DMs — 1:1 chat with inbox, unread counts, privacy controls (Phase 4 slice 5)
_Merged: 2026-05-11_
_Release: v2.2.0_

- [ ] Migration adds `kind` column, `chat_room_members`, `dm_policy`,
- [ ] Boot backfill: every existing `universe_members` row has a
- [ ] `POST /dms/with/:user_id` is idempotent (calling twice returns
- [ ] Canonical pair ordering: A→B and B→A both produce the same slug.
- [ ] `POST /dms/with/:user_id` honors `dm_policy=nobody` → 403.
- [ ] `POST /dms/with/:user_id` honors `dm_policy=shared-universe`:
- [ ] `POST /dms/with/:user_id` honors blocks → 403.
- [ ] `POST /dms/with/:user_id` 400 on self-DM (`caller_id == target_id`).
- [ ] `GET /me/dms` returns threads ordered by `last_message.created_at DESC`.
- [ ] `unread_count` correctly reflects messages with `created_at >
- [ ] `POST /dms/:room_id/read` updates `last_read_at`; subsequent
- [ ] Posting via existing `POST /universes/dm/chat/rooms/{dm-slug}/messages`
- [ ] Blocking a user prevents new DM messages between them (both
- [ ] Unblocking restores message-send ability.
- [ ] "📩 Mensagens" button appears in sidebar when logged in, with
- [ ] Inbox renders threads with preview + relative time + unread count.
- [ ] Click thread → existing chat drawer opens in DM mode (no room rail).
- [ ] Universe member list grows a 📩 icon on hover; click opens DM.
- [ ] Settings modal: privacy radio + block list manager + DM policy
- [ ] Receiving a new DM (via WS) bumps inbox count and re-orders inbox.
- [ ] Marking a thread as read clears the count immediately + on
- [ ] Self-DM attempt shows i18n error inline.
- [ ] All strings PT + EN.
- [ ] `test_open_dm_creates_room` — first call inserts.
- [ ] `test_open_dm_idempotent` — second call returns same room.
- [ ] `test_open_dm_canonical_pair_ordering` — A→B and B→A same slug.
- [ ] `test_open_dm_blocked_403`.
- [ ] `test_open_dm_policy_nobody_403`.
- [ ] `test_open_dm_policy_shared_universe_no_overlap_403`.
- [ ] `test_open_dm_policy_shared_universe_with_overlap_200`.
- [ ] `test_open_dm_self_400`.
- [ ] `test_post_message_in_dm_member_200`.
- [ ] `test_post_message_in_dm_non_member_403`.
- [ ] `test_post_message_when_blocked_403`.
- [ ] `test_list_my_dms_returns_threads_ordered_by_last_message`.
- [ ] `test_unread_count_increments_on_new_message`.
- [ ] `test_mark_read_clears_unread`.
- [ ] `test_block_disables_existing_dm`.
- [ ] `test_unblock_restores_dm`.
- [ ] `test_backfill_inserts_chat_room_members_from_universe_members`.

### CO-196 — Chat moderation — edit/delete own, admin delete any (Phase 4 slice 4)
_Merged: 2026-05-11_
_Release: v2.2.0_

_(no acceptance criteria in spec)_

### CO-194 — Chat WebSocket — live messages + presence (Phase 4 slice 2)
_Merged: 2026-05-11_
_Release: v2.2.0_

- [ ] WS endpoint mounted + reachable under correct universe slug
- [ ] Auth gates match REST (anonymous = 4401, non-member = 4403,
- [ ] After CO-193 REST POST, all WS subscribers of that room receive
- [ ] `presence.join` / `presence.leave` fired correctly with
- [ ] Keep-alive ping every 30s; silent client dropped at 40s.
- [ ] Connect rate-limit: 6th connect/min returns 4429 close.
- [ ] No regression on CO-193 REST behavior.

### CO-197 — co-auto: fast-forward main after successful task so next ticket branches from current state
_Merged: 2026-05-11_
_Release: v2.2.0_

- [ ] After `co-auto --task CO-N` completes a task successfully in
- [ ] Running `co-auto --task CO-(N+1)` immediately after picks up
- [ ] If FF fails (main diverged), the user sees a clear warning and
- [ ] If the task itself fails (cargo test / clippy / build), no FF
- [ ] Existing worktree mode (`--cycle`, `--teams`) behavior is
- [ ] Add an integration test or scripted scenario that runs two

### CO-193 — Chat schema + REST endpoints (Phase 4 first slice)
_Merged: 2026-05-10_
_Release: v2.2.0_

- [ ] `chat_rooms` + `chat_messages` tables created via idempotent
- [ ] On universe creation, a `general` room is auto-inserted.
- [ ] Boot-time backfill inserts a `general` room for every pre-existing
- [ ] `GET /rooms` returns 401 anonymous, 403 non-member, 200 with
- [ ] `POST /rooms` works for owner/admin, 403 for member/viewer.
- [ ] `POST /messages` works for member+, 403 for viewer/subscriber,
- [ ] `GET /messages` paginates correctly via `?before=<id>&limit=N`.
- [ ] Soft-deleted messages return tombstone text + `deleted_at`.
- [ ] Tests: 12+ covering happy-path read/write, every auth gate,

### CO-187 — Drop legacy HS256 handover helper from co-web (CO-186 cleanup)
_Merged: 2026-05-10_
_Release: v2.2.0_

- [ ] `sign_handover_jwt` function removed from `auth.rs`.
- [ ] Doc comment on `sign_handover_jwt_es256` no longer mentions an
- [ ] `cargo build -p co-web` passes.
- [ ] `cargo clippy -p co-web -- -D warnings` clean.
- [ ] `cargo test -p co-web --lib` shows the same pass count as before
- [ ] `grep -r 'sign_handover_jwt\b' co-web/src/` returns zero matches.
- [ ] Smoke: deploy + verify cross-deployment SSO from

### CO-192 — Sidebar consumes unified /me/universes shape — invited badge, discoverable section
_Merged: 2026-05-10_
_Release: v2.2.0_

- [ ] After login, sidebar fetches `/api/v1/me/universes` and renders
- [ ] Empty buckets are not rendered (no dangling section header).
- [ ] Each non-owned item shows a small role chip.
- [ ] Invited section has the 🎁 emoji + count in label, and each row
- [ ] Accepting an invite optimistically removes the row, calls the API,
- [ ] Discoverable section is collapsible; state persists in localStorage.
- [ ] Anonymous users see the existing public-catalog sidebar (no
- [ ] After clone / subscribe / accept / unsubscribe, sidebar refreshes
- [ ] All i18n strings PT + EN.
- [ ] Universe-tree chevron toggle (existing parent_key nesting) still

### CO-183 — POST /api/v1/leads + admin queue (substituir mailto do contato artelonga)
_Merged: 2026-05-10_
_Release: v2.2.0_

_(no acceptance criteria in spec)_

### CO-190 — Passwordless onboarding via email — magic-code sign-in or signup
_Merged: 2026-05-10_
_Release: v2.2.0_

- [ ] Migration v41 creates `onboarding_codes` table + index.
- [ ] `POST /api/v1/auth/onboard-with-email` works for unknown email,
- [ ] `POST /api/v1/auth/onboard-with-email` works for known email,
- [ ] `POST /api/v1/auth/onboard-with-email/verify` with correct code
- [ ] `POST /api/v1/auth/onboard-with-email/verify` with correct code
- [ ] Wrong code 5 times → 410, code locked.
- [ ] Re-using a consumed code → 410.
- [ ] Expired code (>10min) → 410.
- [ ] Tests: 8+ covering happy login, happy create, unknown email rate-
- [ ] UI: "Continuar com email" is the primary login affordance.
- [ ] After verify with `intent='create'`, the user can immediately set
- [ ] No-enumeration semantics of `/forgot-password` are unchanged.

### CO-189 — Invitation UI — settings panel for inviters + public accept page (Phase 3 second slice)
_Merged: 2026-05-10_
_Release: v2.2.0_

- [ ] Universe settings shows "Convidar pessoas" section for owner / admin only.
- [ ] Submitting form with email triggers POST and shows toast on 201.
- [ ] Submitting form with usuario (no `@`) routes to the `usuario` field.
- [ ] Pending list refreshes after a send.
- [ ] `/invitations/{token}` URL renders preview card without auth.
- [ ] Anonymous user clicking "Aceitar" lands on login modal with `?return_to=/invitations/{token}`.
- [ ] Logged-in user clicking "Aceitar" hits API + redirects to the universe.
- [ ] Identity mismatch is detected and surfaced clearly.
- [ ] Expired / consumed states render their own messages, not generic 404.
- [ ] Inbox-count badge in sidebar (optional — mark as bonus).
- [ ] No regression on existing `setupSettingsPanel` / `setupSecurityModal` flows.
- [ ] All new strings have PT + EN entries.

### CO-188 — Universe invitations — schema + REST endpoints (Phase 3 first slice)
_Merged: 2026-05-10_
_Release: v2.2.0_

- [ ] Migration v40 creates `universe_invitations` table + indexes (idempotent).
- [ ] `POST /api/v1/universes/:slug/invitations` works for owner; 403 for non-member; rejects already-member with 409.
- [ ] Email actually sent via Resend when `invited_email` set; verifiable in `flyctl logs` (`Recovery code emailed` shape).
- [ ] `GET /api/v1/invitations/:token` returns the preview shape; 404/410 on missing/expired.
- [ ] `POST /api/v1/invitations/:token/accept` checks caller identity matches invitee; inserts `universe_members` row; marks consumed.
- [ ] Re-accepting same token returns 410 consumed.
- [ ] Tests: 8+ covering happy path (email + usuario + user_id paths), 403 non-owner, 409 already-member, 410 expired/consumed, identity mismatch on accept.

### CO-172 — Quilombo signups become CO accounts — central auth + redirected password reset
_Merged: 2026-05-09_
_Release: v2.2.0_

- [ ] `quilombo_usuarios` registration creates a CO `users` row + sets `linked_co_user_id` (idempotent if linked or email matches existing CO user)
- [ ] Setting an email via `PUT /api/v1/quilombo/perfil` upgrades a previously-unlinked quilombo user to a CO user
- [ ] `backfill_email_recovery_channels` (or a new `backfill_quilombo_recovery_channels`) ensures every linked quilombo user has their email as a verified recovery channel on every boot
- [ ] `/recover?return_to=...` route exists on CO; safelist enforced (only `*.artelonga.com.br` and `quilomboaraucaria.com.br` accepted)
- [ ] Quilombo SPA "Esqueci minha senha" + "Alterar senha" redirect to that route with prefilled identifier
- [ ] `POST /api/v1/auth/reset-password` mirrors the new hash into `quilombo_usuarios.senha_hash` for all linked rows
- [ ] Tests: signup creates CO user, email-set links existing CO user, reset propagates to quilombo hash, redirect allowlist rejects evil hosts

### CO-171 — Modularize the 6k+ line monoliths — storage.rs, app.js, recovery_routes.rs into composable components
_Merged: 2026-05-09_
_Release: v2.2.0_

- [ ] `storage.rs` reduced to < 500 LOC (the `Storage` struct definition + `new()` + a `pub use`)
- [ ] No file in `co-web/src/` exceeds 1,500 LOC
- [ ] `app.js` reduced to < 500 LOC (boot + module imports)
- [ ] No file in `co-web/static/variants/a/` exceeds 1,500 LOC
- [ ] All existing tests still pass without modification
- [ ] CHANGELOG has a `### Refactored` entry per extraction PR naming the moves
- [ ] No public API change (HTTP endpoints, JS globals, `pub fn` signatures)

### CO-165 — Forgot password / change password with verified recovery channels (email · WhatsApp · SMS), encrypted at rest
_Merged: 2026-05-08_
_Release: v2.2.0_

- [ ] `users.email` becomes nullable; new `users.usuario` column added; backfill from email local-part for existing rows
- [ ] `user_recovery_channels` + `recovery_verifications` + `password_reset_tokens` tables created via migration
- [ ] Channel `value` stored encrypted (ChaCha20-Poly1305 envelope); lookup via keyed BLAKE3 hash
- [ ] All 9 endpoints above implemented and gated by appropriate auth
- [ ] Verification code path works for email (SMTP if `CO_SMTP_*` set, else printed to logs in dev mode)
- [ ] WhatsApp + SMS providers stubbed in Phase 1 (TODO in code: real Twilio + Meta API in Phase 2)
- [ ] `forgot-password` always returns 202 regardless of whether the identifier exists (no enumeration)
- [ ] Rate limit: 5 codes / hour / channel; 5 attempts / code; lockout for 15min after 3 wrong codes
- [ ] UI: recovery flow accessible from login modal + from settings; both PT and EN strings
- [ ] 12+ new tests covering: add → verify → reset → login; expired code; wrong code; rate limit; nullable email migration

### CO-169 — Direct notification provider adapters: Resend email + Evolution API WhatsApp
_Merged: 2026-05-08_
_Release: v2.2.0_

- [ ] `ChannelProvider` trait in `notification_providers.rs` with `name()` + `send()` methods
- [ ] `ResendProvider` implements trait; sends email via `POST api.resend.com/emails`; requires `RESEND_API_KEY` + `RESEND_FROM`
- [ ] `EvolutionApiProvider` implements trait; sends WhatsApp text via Evolution API; requires `EVOLUTION_API_URL` + `EVOLUTION_API_KEY` + `EVOLUTION_INSTANCE`
- [ ] Migration v36 adds `telefone TEXT` (nullable) to `quilombo_usuarios` and `channel` + `recipient` columns to `notifications`
- [ ] `emit_event` enqueues channel-specific rows when env vars are set and recipient is known
- [ ] Worker dispatches by `notifications.channel`; existing webhook path unchanged
- [ ] Template rendering via `{{key}}` substitution; at least 3 templates for `quilombo.evento.criado` (email subject, email body, WhatsApp text)
- [ ] Tests: ResendProvider and EvolutionApiProvider both use mock HTTP (no real API calls); template rendering tests
- [ ] When `RESEND_API_KEY` absent: email rows are never enqueued (no silent failures)
- [ ] When `EVOLUTION_API_KEY` absent: whatsapp rows are never enqueued

### CO-168 — Outbound webhook system + notification queue — n8n/Zapier integration point
_Merged: 2026-05-08_
_Release: v2.2.0_

- [ ] Migration v35 adds `webhooks` + `notifications` tables
- [ ] `POST /api/v1/gestao/webhooks` creates a webhook, returns secret once (admin only)
- [ ] `GET /api/v1/gestao/webhooks` lists webhooks with secret redacted
- [ ] `GET /api/v1/gestao/webhooks/:id/deliveries` returns last 100 notification rows
- [ ] `DELETE /api/v1/gestao/webhooks/:id` deletes webhook + cascades notifications
- [ ] `emit_event(event_type, payload)` helper writes a notification row for each matching enabled webhook
- [ ] Webhook worker starts at boot, polls every 5s, delivers with HMAC-SHA256 signature
- [ ] Retry: up to 3 attempts with 5s / 30s / 2min backoff; 4th failure marks `dead`
- [ ] Wildcard event matching: `quilombo.*` and `*` work correctly
- [ ] At least 3 quilombo events wired: `quilombo.evento.criado`, `quilombo.missao.participou`, `quilombo.mensagem.criada`
- [ ] Tests: webhook registration, wildcard matching, HMAC signature, retry state machine — all in-process (no real HTTP)
- [ ] `docs/webhooks.md` documents the event catalogue and n8n setup guide

### CO-167 — Email collection for quilombo users — bridge to CO unified auth
_Merged: 2026-05-08_
_Release: v2.2.0_

- [ ] Migration v34 adds `email TEXT` (nullable, unique) to `quilombo_usuarios`
- [ ] `PUT /api/v1/quilombo/perfil` accepts and validates `email`; returns 409 if already taken
- [ ] `POST /api/quilombo/auth/login` response includes `"missing_email": true` when user has no email
- [ ] `GET /api/v1/quilombo/admin/usuarios` includes `email` per user row
- [ ] Admin stats endpoint exposes `com_email` + `vinculados_co` counts
- [ ] Test: setting email works; duplicate email returns 409; login response flag correct

### CO-164 — Vector index for entries — semantic search inside CO
_Merged: 2026-05-08_
_Release: v2.2.0_

- [ ] `entry_embeddings` table created via migration v10
- [ ] `fastembed` (or equivalent) integrated, model auto-downloaded on first use to `~/.co/models/`
- [ ] Background worker indexes new entries within 1s for batches up to 32
- [ ] `?semantic=…&k=10` returns top-K with `_score` ∈ [0, 1]
- [ ] `/api/v1/search?semantic=…` aggregates across visible universes
- [ ] Hybrid search (q + semantic) outperforms either alone on a benchmark set
- [ ] Re-index on `body_hash` change (boot scan), not on every write of a same-content entry
- [ ] No regression on 329 lib tests

### CO-163 — Mempalace BaseBackend Python shim using /api/v1/blobs
_Merged: 2026-05-08_
_Release: v2.2.0_

- [ ] `MempalaceCoBackend` class exposes `upsert`, `get`, `query`, `delete` matching mempalace's `BaseBackend` ABC signatures
- [ ] `upsert` writes content blob via `/api/v1/blobs`, embedding blob via `/api/v1/blobs`, metadata entry via vault PUT — three calls, parallelizable
- [ ] `get(ids=[...])` reads vault entries by path; resolves `blob_hash` → bytes via `/api/v1/blobs/:hash`
- [ ] `query` returns keyword-search results today; design has a hook (`self._vector_search` no-op) for swapping in CO-164's vector endpoint when it ships
- [ ] `delete` removes vault entries (CAS blobs are content-addressed and shared — never deleted by the shim)
- [ ] Offline tests in `test_mempalace_co.py` pass against a local CO server (`co serve` in a temp universe)
- [ ] `mempalace_co_README.md` documents config + the keyword-only caveat for `query`

### CO-166 — Single sign-on across universe deployments — co.artelonga.com.br as central auth backend
_Merged: 2026-05-08_
_Release: v2.2.0_

- [ ] CO sets session cookie with `Domain=.artelonga.com.br` (env-flagged, default off in dev so localhost still works)
- [ ] CO publishes `/.well-known/jwks.json` so deployments can verify without sharing secrets
- [ ] Sample integration kit at `scripts/co_auth_kit/` with TS + Python adapters
- [ ] ArteLonga deployment uses kit and authenticates through CO; "Entrar" button works
- [ ] Test: log in on `co.artelonga.com.br`, navigate to `artelonga.com.br`, see logged-in state without re-entering creds
- [ ] `oauth_clients` table + admin UI to register a client
- [ ] `/oauth/authorize`, `/oauth/token`, `/oauth/userinfo`, `/.well-known/openid-configuration` endpoints
- [ ] Standard OIDC code flow with PKCE
- [ ] Quilombo deployment integrates: "Entrar com CO" works, dual-mode with legacy login during transition
- [ ] Per-client `redirect_uri` allowlist; per-client scopes
- [ ] All quilombo users linked
- [ ] Quilombo legacy login removed; users authenticate solely via CO

### CO-161 — Visibility gate consolidation — single tower middleware on /api/v1/universes/{slug}/* (replaces 13 per-handler calls)
_Merged: 2026-05-04_
_Release: v2.2.0_

- [ ] Single middleware function exists in `auth.rs`, applied once in `server::build_router`
- [ ] Zero `check_reader_for_entries` calls in entry_routes / relation_routes / reference_routes (the call sites are removed because the middleware does the work)
- [ ] `asset_routes::check_reader` removed, replaced by the same middleware (asset routes inherit the gate)
- [ ] All read paths pass through the gate; all write paths additionally pass through the writer gate
- [ ] 4 new integration tests cover anon/owner/member/stranger × public/private
- [ ] Existing 316+ tests still pass
- [ ] No regression in the empirical privacy verification (`/api/v1/universes/<private>/entries` returns 401 to anon; `<public>` returns 200)

### CO-152 — topologia-co-adapter — load CO universes into topologia-core::Term/Concept
_Merged: 2026-05-03_
_Release: v2.2.0_

- [x] `topologia/crates/topologia-co-adapter/` exists; depends on `topologia-core` (path) and `co` (path: `../../../co/core`)
- [x] `CoLanguagePlane::open(root, code)` returns Ok for `~/projects/topologia/guarani-mbya/`
- [x] `iter_terms()` over `guarani-mbya/` yields ≥ 8 `Term` values (matching the current entry count)
- [x] `get_term("ayvu")` returns the canonical Mbyá entry
- [x] `CoConceptPlane::iter_concepts()` over `~/projects/topologia/concepts/` yields the 4 anchor entries
- [x] Unit test: round-trip a `.md` with a `references` array through serde + deserialize_term, no data loss

### CO-160 — Inline PDF renderer in the SPA — read references without downloading
_Merged: 2026-05-03_
_Release: v2.2.0_

- [ ] PDF.js bundle vendored at `co-web/static/pdfjs/` (versioned), build picks it up
- [ ] Entry view detects `type: reference` + `medium: pdf` + valid `file:` and renders the inline viewer below the markdown body
- [ ] Bytes only fetched when iframe is visible (browser native; verify via dev tools network tab)
- [ ] `Baixar PDF` button explicitly downloads the file with the original filename
- [ ] `Tela cheia` button toggles the iframe to fullscreen via Fullscreen API
- [ ] On a private universe, the iframe URL resolves through the same auth as the entry view (cookie / bearer); no leakage to anonymous viewers
- [ ] Test: view `mbya/refs/GNDicLex.md` on prod and the dictionary renders inline, pageable
- [ ] Lighthouse "Largest Contentful Paint" stays under 2.5s for entry views without PDF (no regression on entries that don't use this)

### CO-159 — INMET moon-phase importer — populate time/moon-phases/<year>/ from portal.inmet.gov.br
_Merged: 2026-05-03_
_Release: v2.2.0_

- [x] `python3 scripts/import-moon-phases.py 2026` writes 48 files (12 phases per quarter × 4) under `time/moon-phases/2026/` — 2026 actually has 50 phases per INMET
- [x] Re-running the same command is a no-op (no phantom changes; sha256 of generated files stable)
- [x] If INMET's table format changes (a heading text shifts, a column reorders), the script fails loudly with a parse error — don't silently produce wrong data
- [x] Cross-year (`2027`) works without code change
- [x] Test fixture: a saved snapshot of the INMET page for 2026 lives in `tests/fixtures/inmet-luas-2026.html` so CI doesn't depend on the live URL

### CO-158 — Reference versioning — work_id + editions[] + primary/secondary source chain
_Merged: 2026-05-03_
_Release: v2.2.0_

- [ ] Manifest accepts `work_id`, `editions`, `primary_source_chain`, `canonical_edition` on the `reference` type
- [ ] `references_meta` table gains `work_id`, `edition_id`, `primary_layer` columns; existing rows backfilled with `work_id = <slug-of-card-key>`, `edition_id = "default"`, `primary_layer = null` (until authored)
- [ ] `GET /references?work_id=<id>` returns every edition row matching the work
- [ ] Re-uploading the same PDF (same sha256) under a different filename detects the duplicate and merges into the existing edition row instead of creating a new one
- [ ] Test: round-trip a card with 3 editions through write → read → assert all 3 appear in `references_meta`

### CO-157 — PDF metadata extraction tool — auto-populate reference cards from source PDFs
_Merged: 2026-05-03_
_Release: v2.2.0_

- [ ] `python3 scripts/extract-pdf-meta.py mbya/refs/PICH0255-T.pdf` writes `PICH0255-T.md` with at minimum `title`, `pages`, `sha256`, `language`, `extracted_at`
- [ ] When `/Info.Title` is absent, the heuristic title-from-first-page picks something better than "Untitled" (or fails loudly)
- [ ] DOI regex catches the standard `10.\d{4,9}/.*` form and populates `doi:`
- [ ] Re-run with `--force` updates the `extracted_*` fields but preserves any human-authored body content (i.e. only the auto-block at the top is replaced)
- [ ] Diff mode (`--out` exists, no `--force`): writes to stderr, exits non-zero if differences exist
- [ ] Test fixture: a tiny stub PDF in `tests/` whose extraction is asserted

### CO-156 — Universal envelope: binary content cards (PDF/image/video metadata) + uniform CRUD telemetry
_Merged: 2026-05-03_
_Release: v2.2.0_

- [ ] `_universe.yaml` parser accepts `properties_per_type` with the per-content-type property map
- [ ] `references_meta` + `references_fts` tables created on per-universe DB v8 migration
- [ ] On every reference-card write, the row is upserted in `references_meta`; sha256 of the bound asset is computed and stored
- [ ] `GET /references?medium=pdf` returns rows from the `mbya` universe matching every PDF in `mbya/refs/`
- [ ] `GET /references/orphan-blobs` lists assets with no `.md` card
- [ ] `GET /references/broken-cards` lists cards whose `file:` doesn't resolve
- [ ] Every site in the kind-table emits one `telemetry_events` row per event
- [ ] `deployment_version` matches `cargo workspace.package.version` at write time
- [ ] `session_id` is the JWT `jti` (or anon-cookie hash for unauthenticated callers)
- [ ] `/co/co/telemetria` admin dashboard surfaces the new event kinds with default 24-hour window
- [ ] Documentation: `docs/telemetry-envelope.md` lists every kind + its `extra` shape

### CO-155 — topologia-mbya-adapter — Arandu lexicon as a LanguagePlane via mbya_lexicon.db
_Merged: 2026-05-03_
_Release: v2.2.0_

- [x] `topologia-mbya-adapter` crate exists; depends on `rusqlite` (with `bundled`) and `topologia-core`
- [x] `MbyaLanguagePlane::open(...)` returns `Ok` for `~/projects/mbya/mbya_lexicon.db`
- [x] `iter_terms()` returns ≥ 4000 terms (matches the lexicon row count)
- [x] `get_term("ayvu")` returns the Dooley entry with `seed_status: NativeConfirmed`
- [x] `concept` field is populated when the optional `concept_map.sqlite` overlay exists; `None` otherwise
- [x] Test fixture: a tiny stub `mbya_lexicon.db` (3 rows) used in CI so the adapter can be tested without copying the production lexicon
- [x] Loud failure if Arandu's schema differs from the pinned version (so we notice when Arandu evolves)

### CO-154 — References as a first-class content type — citations table + searchable excerpts
_Merged: 2026-05-03_
_Release: v2.2.0_

- [ ] `references_index` + `references_fts` tables created on per-universe DB v7 migration
- [ ] On every entry write, references are extracted from frontmatter and body
- [ ] `GET /references?source=meuorixa.wordpress.com` returns the 1 row from `yoruba/terms/ogunte.md`
- [ ] `GET /references?q=orixá+maternidade` returns the matching FTS row
- [ ] `GET /references/orphan-wikilinks` for `yoruba` returns terms mentioned inside ## Referência: sections that don't have entries yet (the backlog)
- [ ] Backfill on first boot populates the table from existing entries
- [ ] Test: round-trip an entry with a `references` array of length 3 through the write path; confirm 3 rows in `references_index`

### CO-153 — Cross-universe entry_relations — `to_universe` column + `co://` URI resolver
_Merged: 2026-05-03_
_Release: v2.2.0_

- [ ] `entry_relations.to_universe` column exists on all per-universe DBs after this deploy
- [ ] `extract_relations` parses `co://` URIs and populates `to_universe`
- [ ] `GET /relations/inbound` returns rows from other universes' `entry_relations` matching `to_universe = <self>` and `to_path = <path>`
- [ ] Backfill job: walk every universe's `entry_relations`, re-run `extract_relations` on every entry, populate `to_universe` for any value that parses as `co://`
- [ ] Test: `concepts/mother.md` shows N inbound `concept` relations from `guarani-mbya/`, `portuguese/`, `yoruba/`
- [ ] Migration is `ensure_column`-safe (per CO-137)

### CO-151 — Real-time delta sync — protobuf SyncDelta over WebSocket with zstd
_Merged: 2026-05-03_
_Release: v2.2.0_

_(no acceptance criteria in spec)_

### CO-147 — Indexable asset metadata layer — list/filter, tags, mime, frontmatter (Phase 2 of CO-145)
_Merged: 2026-05-02_
_Release: v2.2.0_

- [ ] `GET /api/v1/universes/{u}/assets?type=image/*` returns image-only rows
- [ ] `GET .../assets?tag=foo` returns tag-filtered rows
- [ ] `GET .../assets/tags` returns tag counts
- [ ] `POST .../assets/{sha}/tags` adds tags; reflected in next list
- [ ] `entries.upsert` increments refcount for each `![](sha256:...)` reference
- [ ] Nightly GC rebuilds refcount and matches a fresh count
- [ ] Frontmatter index populates on every entry upsert
- [ ] CHANGELOG entry

### CO-150 — SPA lazy-load integration — img/video, asset browser, frontmatter excerpts (Phase 5 of CO-145)
_Merged: 2026-05-02_
_Release: v2.2.0_

- [ ] Board view with 50 image-laden entries paints in < 1 s; images load as user scrolls
- [ ] Video plays on click without prebuffering the whole file
- [ ] Drag-image-onto-editor uploads to `/assets` and inserts `![](sha256:...)` syntax
- [ ] `?excerpt=true` returns frontmatter + 200-char excerpt
- [ ] Asset browser at `/co/{u}/assets` lists all assets with thumbnails
- [ ] Lighthouse "lazy-load images" audit passes
- [ ] CHANGELOG entry; demo screenshot in `docs/`

### CO-146 — Binary asset upload + sha256 content-addressable storage (Phase 1 of CO-145)
_Merged: 2026-05-02_
_Release: v2.2.0_

- [ ] `POST /api/v1/universes/{u}/assets` accepts raw bytes and returns `{sha256, mime, size}`
- [ ] Same content uploaded twice produces same sha256 and one filesystem blob (idempotent)
- [ ] `GET /api/v1/universes/{u}/assets/{sha256}` returns bytes with correct Content-Type
- [ ] `If-None-Match: "<sha256>"` returns 304
- [ ] DELETE with refcount=0 unlinks the file; refcount>0 returns 409
- [ ] Anonymous request to public universe asset succeeds; to private universe returns 401
- [ ] Upload of 5 MB image completes in < 2 s on UAT (matches CO-81 acceptance)
- [ ] Storage path is sharded `<aa>/<bb>/<sha256>` (no flat dir)
- [ ] `assets` table created via `ensure_table` (drift-safe per CO-137)
- [ ] Tests cover: round-trip, dedup, 304, delete-refcount, oversize-rejection
- [ ] CHANGELOG entry under `### Added` for the version bump

### CO-144 — Per-user dados/ panel inside personal universe + deterministic-chain process model (Co/processos/)
_Merged: 2026-05-02_
_Release: v2.2.0_

_(no acceptance criteria in spec)_

### CO-100 — Documentation pass — ARCHITECTURE / OPERATIONS / ONBOARDING reflecting 1.21.x reality
_Merged: 2026-05-02_
_Release: v2.2.0_

- [ ] Three new files at `docs/ARCHITECTURE.md`, `docs/OPERATIONS.md`, `docs/ONBOARDING.md`.
- [ ] Each opens with a one-paragraph "what this is + when to read it" summary.
- [ ] ARCHITECTURE includes at least one Mermaid diagram (component overview).
- [ ] OPERATIONS commands are copy-pasteable and have been actually run by the agent at least once against UAT.
- [ ] ONBOARDING walks a new contributor end-to-end without hand-waving.
- [ ] All three reference each other where appropriate.
- [ ] Repo `README.md` (top-level) gets a "Documentation" section linking the three.

### CO-143 — Deploy the backup-cron Fly app — daily snapshots are NOT running on prod
_Merged: 2026-05-02_
_Release: v2.2.0_

- [ ] First snapshot landed in `s3://artelonga-co-backups/` (one-shot manual run, before any cron setup)
- [ ] Cron path active (Option A or B)
- [ ] Quarterly restore-drill (CO-119) green against the most recent snapshot
- [ ] `docs/OPERATIONS.md` "Backup & restore" section confirmed runbook-ready
- [ ] Incident-response: documented the case where prod content edits exist but no recent snapshot has captured them — current 2026-05-02 state

### CO-142 — Public-universe routing audit + co-dev/co-experience deprecation + content-count reconciliation
_Merged: 2026-05-02_
_Release: v2.2.0_

- [ ] `dev_board::router()` no longer mounts under `/api/v1/universes/`
- [ ] `curl /api/v1/universes/co-dev` returns 200 (metadata) to anonymous, OR co-dev is deleted entirely (Phase C — pick one)
- [ ] `curl /api/v1/universes/<private-key>` returns 404 (not 403, not 200) to anonymous
- [ ] Smoke test extended to verify each documented public universe is reachable
- [ ] `content_count` reflects actual entry count for `template` (≥6)
- [ ] Same recompute applied to all system-owned universes on next boot
- [ ] Either `upsert_entry_row` increments `content_count` atomically OR there's a startup recompute documented in OPERATIONS.md
- [ ] CO-140 seed code removed (or `co-dev`/`co-experience` rows hard-deleted on next boot)
- [ ] Admin dashboard's `/co/co-dev/telemetria` route either retargeted to `/co` work universe or removed
- [ ] Decision documented: epics stay entries vs. epics promote to sub-universes
- [ ] If sub-universes: one-shot migration creates them with `parent_key='co'`
- [ ] Inventory of quilombo* universes committed to repo (e.g., `docs/UNIVERSES.md`)
- [ ] Each surviving quilombo variant has a documented purpose
- [ ] Stale variants (qb test 2, etc.) hard-deleted
- [ ] Dev board reads from a path that matches the repo's `work/co/CO-*.md`
- [ ] Tasks I closed this session (CO-100, CO-103, CO-104, CO-105, CO-106, CO-107, CO-137, CO-138, CO-139, CO-69, CO-71, CO-74, CO-77, CO-117, CO-119, CO-121, CO-138) show as `done` in the SPA's CO dev board view

### CO-105 — Admin telemetry dashboard — /api/v1/admin/dashboard + /admin static page
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] `co-web/src/admin_routes.rs` (or extension of existing) implements the JSON endpoint.
- [ ] Route gated by `email == CO_SEED_ADMIN_EMAIL` (read fresh from env each request — no caching of the gate).
- [ ] Storage helpers return the aggregates with proper typing.
- [ ] Static `co-web/static/variants/a/admin.html` renders the data.
- [ ] yuri can log in, navigate to `/admin`, and see real numbers from prod.
- [ ] Non-admin user navigating to `/admin` sees 403 (not 404 — meaningful for debugging).
- [ ] Rust tests for the aggregate queries + access gate.

### CO-121 — A/B primitives on OLTP (feature_flags, ab_assignments, ab_exposures)
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] Migration adds the three tables; rolls back cleanly
- [ ] `co_ab::assign` returns the same variant for 1000 consecutive calls with the same input
- [ ] `co_ab::expose` writes to OLTP **and** emits a WAE row within 60s
- [ ] Flag definitions seedable from `data/seed/feature_flags.yaml`
- [ ] Admin endpoint `POST /api/v1/admin/flags` to create/toggle flags
- [ ] One real flag wired up end-to-end (e.g., `home_v2_layout`) and exposed events visible in WAE

### CO-123 — ClickHouse single-node on Fly + Iceberg-table-function ready
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] Fly app `co-clickhouse` deployed with 4 vCPU / 8 GB / 50 GB Volume
- [ ] Internal-only access (Fly private network or `fly proxy` for queries)
- [ ] Daily WAE → ClickHouse export job (Fly Machine cron)
- [ ] Sample queries documented: top-10 universes by activity (24h), error rate by deploy, retention 7d/30d
- [ ] `iceberg(s3, '...')` table function enabled and tested against an empty Iceberg table on R2 (smoke test for Phase 3 readiness)
- [ ] `docs/OPERATIONS.md` updated with how to run a query, where to read results, how to back the volume up

### CO-141 — Meaning-topology dictionary — concept + language universes (mbyá / portuguese / yoruba)
_Merged: 2026-05-01_
_Release: v1.0.0_

- [x] `~/projects/topologia/README.md` exists and explains the model + relationship to Arandu.
- [x] Four universe directories under `~/projects/topologia/`, each with `_universe.yaml` + `index.md` + term/concept entries.
- [x] **Cross-language chain** demonstrable: `concepts/mother.md` ← {`guarani-mbya/xy.md`, `jaryi.md`, `portuguese/mae.md`, `yoruba/iya.md`}.
- [x] **Within-language compound** chain: `guarani-mbya/pyau.md` ↔ `jaxy-pyau.md` ↔ `ara-pyau.md`.
- [x] **Both-axes chain**: `concepts/word.md` ↔ {`guarani-mbya/ayvu.md`, `nhe-e.md`, `portuguese/palavra.md`, `yoruba/oro.md`}.
- [x] At least one term demonstrates the **references + linked content** pattern with quoted external source body + embedded `[[wikilinks]]` (`yoruba/terms/ogunte.md`).
- [x] Every term file carries `seed_status: draft`.
- [x] No Rust changes in co; no changes to existing co universes; no version bump in co.

### CO-124 — Co-agent variants for CF Workers tail + Vercel Log Drains
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] CF tail Worker repo published, deployable via `wrangler deploy`, with documented setup
- [ ] Vercel Log Drain route accepts and processes the Vercel test payload
- [ ] Sample deployments on both platforms ship ≥99% of emitted log lines to the CO ingest within 60s
- [ ] Both variants reuse the same HMAC + zstd + ingest URL contract as CO-120
- [ ] `docs/co-agent/cloudflare-workers.md` and `docs/co-agent/vercel.md` published

### CO-97 — Unify telemetria visitor token (visitante_id) with marketing al_vid (#23)
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] Decision documented in `docs/decisions/0XX-visitor-token-unification.md` (option A/B/C, rationale, trade-off)
- [ ] If A or B: HttpOnly trade-off reviewed, signed off, captured in `data/universes/template/content/dados-rastreados.md` so users see the change
- [ ] `telemetry_middleware` reads `al_vid` first when present, falls back to `visitante_id`
- [ ] Co's cookie scoped at apex (`.artelonga.com.br`) when serving from `co.artelonga.com.br`
- [ ] Marketing site cookie scoped at apex
- [ ] One visitor → one token, validated end-to-end (load marketing → load co → assert both surfaces report the same token)
- [ ] No regression in existing telemetry queries (visitor count not double-counted during compat window)
- [ ] CSP review on both subdomains; document any tightening

### CO-120 — Co-agent adapter trait + Fly sidecar reference impl
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] `CoAgent` trait + `FlySidecarAgent` impl in `co-core`
- [ ] Standalone `co-agent` binary builds for `aarch64-unknown-linux-musl`
- [ ] Docker image `co-agent:0.1.0` published to GHCR
- [ ] Reference compose snippet in `docs/co-agent/fly-sidecar.md` showing how to add it as a sidecar to a Fly Machine
- [ ] Unit tests cover: batch flush at size threshold, batch flush at time threshold, retry-then-drop on persistent 5xx, HMAC validation
- [ ] One end-to-end test: stand up sidecar against UAT ingest, emit 100 events, confirm ≥99 land

### CO-122 — Quota/tier model spec (no enforcement yet)
_Merged: 2026-05-01_
_Release: v1.0.0_

- [x] `docs/QUOTAS.md` written with the tier matrix + behaviors
- [x] Reviewed by yuri (with input from the SR engineer per §F.6 of the review)
- [x] No code changes — this is a spec doc only
- [x] Linked from `docs/README.md` and `work/co/SPRINT-V1-LAUNCH.md`

### CO-118 — Workers Analytics Engine ingest from co-web for exposure + telemetry events
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] WAE dataset `co_telemetry` provisioned
- [ ] Worker proxy deployed at `wae.co.artelonga.com.br` (or as a route on the apex zone) with API-key auth
- [ ] `co-web` `telemetry` module gains `emit(TelemetryEvent)` helper using async fire-and-forget
- [ ] Synthetic exposure event from UAT visible in WAE SQL within 60s
- [ ] Privacy filter test: payload with a 1KB `body` field is rejected with a structured error
- [ ] Existing telemetry tables in OLTP keep working (parallel write, not replacement)

### CO-109 — Mbya Guarani stress-test universe — lexicon PDF → markdown corpus + universe seed
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] `scripts/lexicon-to-markdown.py` produces N markdown files under `~/projects/mbya/content/lexicon/`
- [ ] Each file has valid frontmatter with the documented fields
- [ ] No filename collisions (slug uniqueness verified)
- [ ] N ≥ 1,000 entries (otherwise the corpus is too small to stress-test)
- [ ] `mbya` universe exists on UAT with N entries
- [ ] Anonymous browser can load `/co/mbya` and see the entries
- [ ] Theme `scholarly` applied; layout = table
- [ ] yuri is owner; pages render correctly via Conteúdo + frontmatter preview
- [ ] CO-101 has a `06-mbya-browse.js` k6 scenario
- [ ] Baseline run captures p50/p95/p99 against mbya at 50/100/500 VU
- [ ] At least one production-shape regression caught by mbya is documented (e.g., "without index on entries.frontmatter_json->>'lema', list-by-lema is O(N)")
- [ ] CO-108's `scripts/backup-to-disk.sh` includes `mbya` in the SOURCES map
- [ ] First archive run produces `mbya.tar.zst` ≤ 25 MB

### CO-108 — Universe archive format + backup-to-external-HD — co-compatible, storage-optimized snapshots of all source repos
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] `scripts/backup-to-disk.sh /Volumes/Backup --from local` produces five `.tar.zst` files (one per universe) at `/Volumes/Backup/co-archive/<YYYY-MM-DD>/`
- [ ] Each archive contains `manifest.json`, `entries/`, `co.db`, `README.md`
- [ ] Total size ≤ 50 MB for the four content universes (mbya excluded — depends on CO-109)
- [ ] Restore: `bash scripts/restore-from-disk.sh <archive> /tmp/test-co` → spinning up `co-web` against `/tmp/test-co/data` boots with the archived universe live and addressable at `/co/<key>`
- [ ] `manifest.json` has a verifiable SHA256 of the bundle
- [ ] `--from prod` ssh-extracts `co.db` and `rsync`s `/data/universes/`, captures actual deployed state
- [ ] `--from uat` does the same against UAT
- [ ] Restore from a `--from prod` archive into a fresh local Co reproduces the prod sidebar exactly
- [ ] `scripts/backup-to-disk.sh --verify <date>` re-hashes archives and compares against manifest — fail loud on bit-rot
- [ ] Optional weekly cron (mac launchd or simple `at`) — operator-driven, documented in `docs/OPERATIONS.md`
- [ ] `docs/OPERATIONS.md` gets a "Local-source backup" section linking to this script

### CO-80 — Per-tier rate limiting + quota — token bucket per user/tier/operation
_Merged: 2026-05-01_
_Release: v1.0.0_

_(no acceptance criteria in spec)_

### CO-79 — Caching layer — manifest, theme.css, hot queries + CDN strategy
_Merged: 2026-05-01_
_Release: v1.0.0_

_(no acceptance criteria in spec)_

### CO-77 — Per-universe SQLite + global metadata DB + LiteFS read replicas
_Merged: 2026-05-01_
_Release: v1.0.0_

_(no acceptance criteria in spec)_

### CO-69 — PWA offline — IndexedDB cache + Background Sync (INFRA-4)
_Merged: 2026-05-01_
_Release: v1.0.0_

_(no acceptance criteria in spec)_

### CO-101 — Load test scaffolding — k6 scenarios + baseline against UAT (#18)
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] Three k6 scripts under `tests/load/`.
- [ ] Each script aborts when `BASE_URL` looks like prod.
- [ ] `tests/load/README.md` explains how to install k6 (`brew install k6`), run a scenario, read the output, and update the baseline.
- [ ] Baseline file committed with results from at least 50-VU runs of each scenario.
- [ ] Documented the failure mode at 500 VUs (where it broke, exact symptom).
- [ ] One section in baseline file recommends what we need to test against before opening v1.0 to the public — concrete number (e.g. "200 VUs sustainable on the current Fly machine; recommend bumping to shared-cpu-2x before public launch").

### CO-104 — Backup automation — daily snapshot of SQLite + universes/ to S3, with tested restore (#13)
_Merged: 2026-05-01_
_Release: v1.0.0_

- [ ] Both scripts exist, executable, with the safety guard on `restore.sh`.
- [ ] Bucket exists with lifecycle policy.
- [ ] Manual run of `backup-prod.sh` produces two objects in the bucket.
- [ ] Restore test against UAT completes successfully and the restored UAT serves the prod data.
- [ ] Cron is scheduled and at least one automated run has succeeded.
- [ ] `docs/OPERATIONS.md` has a "Backup & restore" section linking to both scripts.

## Carried Over

- (none tracked — retrospective simulation uses merge commits only)
