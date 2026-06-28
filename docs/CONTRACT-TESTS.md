---
type: doc
title: Contract Tests — the bot↔co-web seam tier
---

# Contract Tests (CO-520)

The test tier that pins every **cross-process seam** between a client and co-web
against the **actual co-web source**, so a request/response shape mismatch fails
CI instead of silently breaking in production.

Today the only client with a contract suite is the WhatsApp companion bot
(`~/projects/tools/whatsapp-bot`, `tests/test_co_contract.py`). The pattern is
client-agnostic; new clients (a CLI, a mobile app, Yggdrasil) should grow their
own contract file following the same rule.

## What it is — the missing middle tier

```
        ┌─────────────────────────────────────────────┐
        │ browser-e2e (Playwright)                     │  browser → co-web
        │   co-web/e2e/*.spec.ts                       │  (the bot seam is
        │   prod-usability gate (CO-421)               │   NEVER crossed here)
        ├─────────────────────────────────────────────┤
   ►    │ CONTRACT  ← THIS TIER (CO-520)               │  bot  ↔ co-web
        │   tools/whatsapp-bot/tests/test_co_contract  │  (the actual wire shape,
        │                                              │   pinned vs Rust source)
        ├─────────────────────────────────────────────┤
        │ unit-with-fakes                              │  each side, in isolation
        │   bot: tests/test_data_rights.py (FakeCo)    │  (a fake re-encodes the
        │   co-web: #[cfg(test)] mod tests             │   wire shape — both green
        │                                              │   even when they disagree)
        └─────────────────────────────────────────────┘
```

The contract tier sits **between** unit-with-fakes and browser-e2e. It is the
only tier that crosses the bot↔co-web process boundary while still running in CI
(no live server, no browser): it drives the **real client method**, captures the
**actual HTTP request** it builds (method / path / body / query), and asserts
that shape against the **real co-web Rust source on disk** — the request DTO's
field names, the route's path+method, the required literal phrases.

## Why — the lib-vs-integration gap that let CI go green while prod broke

Every cross-process bug found in the WhatsApp wave review shipped **green**:

| Bug | What broke | Why both unit tiers passed |
|---|---|---|
| **CO-492** erase/forget body | bot POSTed a bot-internal `confirm_token` nonce; co-web's `ConfirmBody` only reads `confirm` and compares it to the literal phrase → every destructive op `400`ed | bot's `FakeCo` accepted the nonce; co-web's unit test fed the *phrase*. Neither crossed the seam. |
| **CO-518** digest window | bot sent `?window=30d`; co-web's `DigestQuery` only deserialized `days`, so the window was silently dropped and the span ignored | bot test asserted it built `window=`; co-web test fed a `days` value. Both green. |
| **CO-490** link/identity | the loopback `{whatsapp, token, user_id}` body — a rename on either side would 400 the link | co-web built the body in a Rust test; the bot decoded a hand-written dict in a Python test. |

The common shape: **both sides were unit-tested with fakes that re-encoded the
same wrong wire shape.** A fake is, by definition, the author's *belief* about
the other side — so two fakes can agree with each other and both be wrong. The
browser-e2e tier never exercises the bot, so nothing caught the disagreement
until prod.

A contract test removes the fake from one side of the seam: it reads the **other
side's source** as the source of truth. It fails the moment the two sides drift.

## The rule

> **Every bot↔co-web seam is pinned against the co-web source.**
>
> For each client→co-web call, a contract test asserts BOTH halves:
>
> 1. **Wire shape (client side).** Drive the real client method, capture the
>    request it builds, assert the exact method, path, body field *names* (and
>    any required literal *values* — e.g. the confirm phrases), and query params.
> 2. **Source pin (co-web side).** Read the co-web Rust file from disk and assert
>    it still decodes exactly what the client sends — the request DTO's fields,
>    the route registration, the phrase constants. When the co-web checkout is
>    not on disk (a CI image without the sibling `co` repo), the source-pin half
>    degrades to a printed skip so the wire-shape half still runs and the suite
>    stays portable.
>
> A drift on **either** side — the client renames a body key, or co-web renames
> the field it deserializes — must FAIL a test.

Pin literals (confirm phrases, route strings) as **independent constants** in the
test, never `import`ed from the client module. Importing them would re-encode
whatever the client currently sends and defeat the purpose; the whole point is
that the test is a third party that compares the two sides.

## Seams currently pinned

`tools/whatsapp-bot/tests/test_co_contract.py` (21 tests) covers, for each
`bridge/co_tools.py` method and the link loopback:

| Seam | Client call | co-web target | Pinned |
|---|---|---|---|
| **me/export** | `me_export` | `whatsapp_me.rs` `export_handler` | `GET /api/v1/whatsapp/me/export`, no body, route is `get()` |
| **me/erase** | `me_erase` | `whatsapp_me.rs` `ConfirmBody` + `ERASE_CONFIRM_PHRASE` | `POST`, body `{"confirm":"apagar tudo"}`, never a nonce |
| **me/forget** | `me_forget` | `whatsapp_me.rs` `FORGET_CONFIRM_PHRASE` | `POST`, body `{"confirm":"esquece de mim"}`, never a nonce |
| **telemetry/digest** | `telemetry_digest` | `lead_digest.rs` `DigestQuery` + `Digest` | `?universe=&days=<int>` (never the dropped `window=`); reads `named`/`aggregate` |
| **consent** | `consent` | `whatsapp_consent.rs` `ConsentQuery` | `GET …/consent?operator=` |
| **onboard** | `onboard` | `onboarding_routes.rs` `OnboardRequest` | `POST …/onboard-with-email` `{email}` |
| **verify** | `verify` | `onboarding_routes.rs` `OnboardVerifyRequest`/`Response` | `{email,code}`; reads `user_id` |
| **create_universe** | `create_universe` | `universe/routes.rs` `CreateUniverseRequest` | `{key,name,visibility,parent_key}` |
| **create_entry** | `create_entry` | `dto/entries/create_request.rs` `CreateEntryRequest` | `{path,frontmatter,body}` (type/title inside frontmatter) |
| **update_entry** | `update_entry` | `dto/entries/update_request.rs` `UpdateEntryRequest` | `PUT`, `{body}` |
| **outbound_relations** | `outbound_relations` | `dto/relations/query.rs` `RelationQuery` | `?path=` |
| **list_universes** | `list_universes` | `content/models.rs` `MeUniversesResponse` | reads `owned`/`member`/`subscribed` |
| **link/identity** | `bridge/main.py` `handle_identity` | `whatsapp_cloud_routes.rs` `build_identity_push` | loopback body `{whatsapp,token,user_id}`, token never echoed |

Run it (stdlib only, no server, no pytest required):

```bash
cd ~/projects/tools/whatsapp-bot
python3 tests/test_co_contract.py        # → "ok — 21 CoTools<->co-web contract tests passed"
# or the whole bot suite:
for f in tests/test_*.py; do python3 "$f"; done
```

## How to add a contract test

When you add (or change) a client→co-web call:

1. **Find the co-web target.** The route registration + the request DTO struct
   it deserializes (and any response field the client reads). Note the file path
   under `co-web/src/`.
2. **Pin the wire shape.** Drive the real client method through the recording
   transport (`_recording_co()` returns `(client, calls)`; each call is
   `(method, path, body)`), then assert the method, path, exact body keys, and
   query params.
3. **Pin the source.** Use the helpers in `test_co_contract.py`:
   - `_assert_decodes(label, relpath, struct_name, sent_keys)` — every key the
     client sends must be a field co-web's `struct_name` deserializes.
   - `_assert_route(label, relpath, needle)` — the route registration string
     (e.g. `post(erase_handler)`) must appear in the file.
   - `_coweb_phrase(const_name)` / `_struct_fields(src, name)` for literal
     constants and response-field pins.
4. **Pin literal values independently.** If the seam requires an exact string
   (a confirm phrase, an enum value), assert it as a top-level constant in the
   test AND cross-check it against the co-web source — do not import it from the
   client.
5. **Keep it portable.** Source-pin helpers must skip (printed) — never error —
   when the `co` checkout is not on disk, so the suite runs anywhere the client
   does.

## Relationship to the rest of the pipeline

- This tier is a **pre-merge** guard. It runs with the client's normal test
  command — no server, no browser — so it belongs in the same fast lane as the
  unit tiers. See the CI failure-cause playbook: [`docs/ci-cd.md`](ci-cd.md)
  ("failure causes & proactive prevention").
- It does **not** replace `cargo test` (co-web's in-process unit tests) or the
  browser-e2e / prod-usability gate ([`docs/e2e-walkthrough.md`](e2e-walkthrough.md),
  [`docs/delivery-pipeline.md`](delivery-pipeline.md)) — it fills the gap
  *between* them: the cross-process seam those two tiers structurally cannot see.
- The lesson it encodes (two fakes can agree and both be wrong) generalizes to
  any seam where two independently-tested processes exchange a wire format. Pin
  the seam against the *other side's source*, not against your own fake.
