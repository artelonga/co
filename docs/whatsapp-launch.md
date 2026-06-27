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

---

## UX north star — an amicable journey (general)

The front door is for someone whose entire digital comfort zone is WhatsApp, yet
who is **eager to understand the technology underneath** — and who deserves real
privacy and data autonomy *because of*, not despite, that. Two commitments:

1. **Everything happens inside WhatsApp.** No website, password, download, or
   copy-pasted token — ever. The one code the user touches is the OTP, which
   arrives *in WhatsApp itself* (a trust primitive they already own).
2. **Tell the truth in rungs; never use a dark pattern.** The privacy story isn't
   a policy they'll never read — it's experiences they can feel and verify.

The arc (hospitality → belonging → understanding → autonomy → reciprocity):

- **Useful before any ask** — read-only help with no account (reframed as
  hospitality, not a limitation), and a plain statement of what the bot is/isn't.
- **The link *is* the consent moment** — "um cantinho só seu", confirmed by the OTP
  on their own WhatsApp. The exact words they agree to are the deterministic,
  versioned consent text (below), recorded as the LGPD record.
- **The everyday** — they talk (and speak — voice in/out, transcribed locally is
  the biggest accessibility+privacy unlock, CO-494); the graph/universe machinery
  is never named ("seu jardim").
- **The curtain lifts — progressive disclosure** (CO-495): optional "look-behind"
  doors, each teaching one real concept — *local AI* ("como funciona"), *data you
  can hold* ("mostra minhas coisas" returns their file), *open source* ("a receita
  é aberta"), *no lock-in* ("você leva seu jardim e ele continua funcionando").
- **The three sacred commands** (CO-493), always honored, no friction on exit:
  `mostra minhas coisas` (export), `apaga tudo` (erasure), `esquece de mim` (revoke
  the bot's per-user token).
- **Reciprocity** — the eager learner becomes a co-creator who brings the next
  person in (`ñandé`, not `oré`).

**The honest knot:** we deliver autonomy *through* Meta's WhatsApp — content to a
Business-API number is not E2E-private. We **name this** (the `boundary` text) and
treat WhatsApp as the on-ramp, not the destination, widening the user's world
toward a more private door over time. Honesty here is the privacy feature.

**North star ≠ engagement.** Success is: the user *feels ownership*, *understood a
little of the magic*, *trusts because they verified*, **could leave and chose to
stay**, and **brought someone in.** Autonomy, comprehension, dignity, community.

### Consent & privacy are deterministic and versioned (CO-491)

Consent, privacy, rights, and the sacred-command confirmations are **legal
artifacts, not conversational copy**. The LLM phrases *help*; it **never** phrases,
paraphrases, translates, or warms up the agreement. The single source of truth is
the versioned file **`co-web/seed/legal/whatsapp-consent.<version>.yaml`**, shown
**verbatim**; the version is recorded against the user's consent so the exact text
they agreed to is reproducible (LGPD Art. 8º §2º — demonstrabilidade). Changing
wording means minting a **new version**; published versions are never edited in
place. The bot's `bot/profile` (see `docs/whatsapp-bot-profile.example.json`)
routes these intents to the deterministic strings *before* the model loop.

### LLM-curated content is the user's data (CO-492)

Anything the assistant writes or organizes **for** a user is **that user's data**
under LGPD — persisted in their space, included in `mostra minhas coisas` (export)
and `apaga tudo` (erasure), and removed when they revoke. The model is a lens over
the user's substrate, never an owner of what it produces for them.

> Release work items: **CO-489/490 (done)**, **CO-491** (deterministic consent),
> **CO-492** (LGPD retention/export of curated content), **CO-493** (sacred
> commands), **CO-494** (local voice I/O), **CO-495** (warm profile + disclosure
> rungs), **CO-496** (tier-3 scalability). See `work/co/CO-49*.md`.

---

## Deployment modes — self-host always possible, or managed-with-consent (CO-497)

Two guarantees hold for the whole release:

1. **You can ALWAYS self-host the whole thing** — your data on your box, nothing
   required from our infra or any third party.
2. **Or opt into managed infra** knowing your data is yours, safe, and private — we
   collect nothing (e.g. telemetry) without explicit consent under a *specific,
   versioned* policy. This includes sensitive data (WhatsApp number, secrets) and
   identifying data (name, CPF) you choose to keep as an instance-local variable.

| | **Self-host** (your box) | **Managed** (our infra / your public host) |
|---|---|---|
| WhatsApp transport | **Evolution** — QR-link your own WhatsApp, outbound-only, **no Meta app, no public webhook** (survives residential CGNAT) | **Cloud API** — Meta app + token + public HTTPS webhook |
| Inbound | Evolution → bridge `/webhook` (LAN/loopback, no open port) | Meta → co-web `/api/v1/whatsapp/webhook` |
| Sends (OTP/reply/greeting) | the **same** `whatsapp_provider_cascade()` picks Evolution | …picks Cloud API |
| Model | local Ollama (default) | local Ollama; opt-in Claude spill (policy update required) |
| Data (tokens, consent, content, name/CPF) | **all local**, encrypted at rest | yours; never collected without consent under a specific policy |
| Public domain | optional — **Cloudflare Tunnel** if you want one (CGNAT-proof, TLS); not needed for LAN/private | native |
| Ops you own | autostart (launchd), live backup (Litestream → B2/S3), UPS | ~none |

**The code is identical across modes** — `notification_providers::whatsapp_provider_cascade()`
(Cloud → Evolution) means a self-hosted deploy with only Evolution behaves exactly
like a managed Cloud deploy; no fork, no special build.

**Data residency invariant:** sensitive (WhatsApp number, secrets/tokens) and
identifying (name, CPF) fields are stored **on the instance**, encrypted where
applicable, and are **never** sent to our infra or a third party in the default
path. On managed infra, anything we'd collect (telemetry, diagnostics) is **opt-in
under its own versioned policy** — default OFF, distinct from the core consent
(CO-491). A "local variable" stays local.

> Self-host ops files (launchd plist, run/import scripts, OrbStack compose,
> `cloudflared` example) are the operator's choice for a *public* self-host — a
> private/LAN Evolution deploy needs none of them. They live with the deploying
> repo, not the app.
