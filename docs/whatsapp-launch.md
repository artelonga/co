# WhatsApp bot — launch runbook (CO-479/480/481/487/489)

How the WhatsApp path is wired, how to test it locally (cheap → live), what to
set to go live, and the gaps to close before a real user touches it.

> Companion code: `tools/whatsapp-bot/` (the `bridge`, local-only, stdlib). The
> Rust glue lives in `co-web/src/integrations/whatsapp_cloud_routes.rs` and
> `bot_proxy_routes.rs`.

## The path (end-to-end, tenant-aware)

```
Meta → POST /api/v1/whatsapp/webhook        (HMAC verify, fast 200, async spawn)
     → parse_inbound → {from, text, phone_number_id}
     → CO_BOT_BRAIN_URL  (bridge /api/chat)  forwarding phone_number_id
     → BrainRouter: phone_number_id → universe → bot/profile → LifecycleBrain
        (Claude tool-loop over CoTools, Ollama phrases) → {reply, intent}
     → CloudApiProvider.send(from, reply) → WhatsApp

companion app → POST /api/v1/bot/chat {text, universe?}  (auth) → same brain
```

One bot deploy serves every customer: the inbound business number
(`phone_number_id`) selects the tenant universe; that universe's `bot/profile`
entry configures persona + enabled capability packs. Onboarding a customer is a
config edit + a profile entry — **zero code**.

## Test ladder (cheapest → live)

| # | Layer | How | Status |
|---|---|---|---|
| 1 | Unit | `cargo test -p co-web whatsapp` (parse_inbound / parse_reply / verify_signature) + `tools/whatsapp-bot` Python tests | ✅ |
| 2 | Bridge ↔ CO | `scripts/e2e-lead-lifecycle.sh` — full lead lifecycle vs local `CO_ENV=local` co-web | ✅ 19/19 |
| 3 | Brain live | `ANTHROPIC_API_KEY` + Ollama + local co-web → `curl <bridge>/api/chat` a real pt-BR ask; assert it creates universe/entries | ⏳ needs key+Ollama |
| 4 | co-web → brain (no Meta) | **`scripts/whatsapp-webhook-smoke.sh`** — posts a self-signed Cloud payload, asserts fast 200, `phone_number_id` forwarding, forged-sig 401, GET handshake | ✅ 7/7 |
| 5 | Tenant routing | `TENANT_REGISTRY={"1238594552665575":"miguel"}` + seed `miguel` `bot/profile`; that number → Miguel's persona, another number → generic concierge | ⏳ |
| 6 | Live Meta | Real token + GET verify handshake + send a real message | ⏳ Meta paste |

Run the key integration test (Layer 4) — it needs only a built `co-web` and
`python3`, no Meta, no key:

```bash
bash scripts/whatsapp-webhook-smoke.sh   # rebuild co-web first if it's stale
```

> ⚠️ The smoke only `cargo build`s co-web if the binary is **missing**. After any
> change to `whatsapp_cloud_routes.rs`, rebuild first (`cargo build -p co-web`) —
> a stale binary silently drops `phone_number_id` (the symptom: text forwards but
> the tenant id arrives empty).

## Launch checklist

**co-web secrets** (`flyctl secrets set … -a co-artelonga`):

| Secret | Value |
|---|---|
| `WHATSAPP_CLOUD_TOKEN` | Meta access token (the bot's WhatsApp send credential) |
| `WHATSAPP_PHONE_NUMBER_ID` | `1238594552665575` |
| `WHATSAPP_VERIFY_TOKEN` | any shared string; echoed during the GET handshake |
| `WHATSAPP_APP_SECRET` | Meta app secret; verifies inbound POST signatures |
| `CO_BOT_BRAIN_URL` | reachable bridge `/api/chat` (bridge stays loopback-only; co-web calls it) |

**Bridge env** (`tools/whatsapp-bot`):

| Env | Value |
|---|---|
| `ANTHROPIC_API_KEY` | Claude orchestrator key |
| `OLLAMA_URL` | reachable Ollama (phrasing) |
| `CO_V1_URL` | co-web base (the `/api/v1/*` server) |
| `CO_TOKEN` | the bot's durable CO service token (see Gap #1) |
| `TENANT_REGISTRY` | inline JSON `{"<phone_number_id>": "<universe>"}` (or `TENANT_REGISTRY_FILE`) |
| `CO_DEFAULT_UNIVERSE` | fallback / single-tenant universe |

**Meta:** webhook URL (HTTPS) subscribed to `messages`; the GET verify handshake
must pass (`hub.verify_token` == `WHATSAPP_VERIFY_TOKEN`).

**Per customer (zero code):** add a `TENANT_REGISTRY` entry mapping their number →
their universe, and seed a `bot/profile` entry in that universe:

```json
{
  "persona": "Você é o tutor de SAT do Miguel...",
  "phrasing_model": "qwen2.5:14b-instruct",
  "capabilities": ["lifecycle", "tracking"],
  "config": { "tracking": { "log_type": "sat-score" } }
}
```

Absent/unreadable profile → the generic CO concierge (lifecycle only).

> **Consistency:** the `TENANT_REGISTRY` key, `WHATSAPP_PHONE_NUMBER_ID`, and
> Meta's inbound `metadata.phone_number_id` must be the **same** id (they are —
> `1238594552665575`).

## Gaps to close before a real user (review-surfaced)

1. **Bot CO identity in production — the genuine blocker beyond the Meta paste.**
   `CoTools` now authenticates with a Bearer **`CO_TOKEN`** when set (durable
   service session); without it, it falls back to the dev onboard→dev_code→cookie
   flow, which is local-only. **Action:** mint a long-lived token for the bot's CO
   user (`POST /api/v1/auth/token` after a password-login) and set `CO_TOKEN` on
   the bridge. *(Wired — `bridge/co_tools.py` `_headers`; verified by
   `tests/test_co_tools_auth.py`.)*
2. **Cross-tenant authorization.** That service identity must be permitted to
   create entries in each tenant universe (member/role), **or** the bot acts only
   in universes it owns/subscribes. Confirm co-web's permission model allows the
   bot user to write to each customer universe before onboarding paid tenants.
3. **Degrade leakage.** With no `ANTHROPIC_API_KEY` the brain currently replies
   `"…precisa de ANTHROPIC_API_KEY…"` — fine in dev, but that string would reach a
   real user. Before live, gate the *reachable-but-unconfigured* case to a generic
   ack. (The webhook already falls back to a fixed ack when the brain is
   *unreachable*; this is the different, configured-but-keyless case.)

## Going live (Layer 6)

1. Set the five co-web secrets + redeploy; set the bridge env (incl. `CO_TOKEN`).
2. In Meta, point the webhook at `https://co.artelonga.com.br/api/v1/whatsapp/webhook`,
   subscribe to `messages`; the GET verify handshake should pass.
3. Send a WhatsApp message to the test number from an allow-listed recipient;
   confirm the reply. Watch `flyctl logs -a co-artelonga` for `WhatsApp Cloud inbound`.
