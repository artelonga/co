# WhatsApp Cloud API — enterprise-compliant integration (CO-479)

CO has **two** WhatsApp channels behind the same `ChannelProvider` trait:

| Provider | Transport | Use |
|---|---|---|
| `EvolutionApiProvider` | Evolution/Baileys (WhatsApp-Web, **unofficial**) | dev / personal bot, prototyping. **Against WhatsApp ToS** — ban risk, no SLA/DPA. |
| `CloudApiProvider` | Meta **WhatsApp Business Platform** (Cloud API / Graph API) | **Production / enterprise** — verified WABA + number, compliant. |

This runbook covers standing up the **compliant** Cloud API path.

## 0. Why it's a different shape than the local bot

The Evolution bot links as a WhatsApp *device* (outbound socket) → no public
webhook. The Cloud API **inverts** this: **Meta calls your webhook**, so you need
a **public HTTPS endpoint** (`co-artelonga` on Fly, or a tunnel for dev). The
compliant path also requires Meta-approved **templates** for business-initiated
messages and documented **opt-in** (LGPD).

## 1. Meta setup (the long pole — start early)

1. **Meta Business Manager** account → **business verification** (CNPJ docs).
   This review takes calendar time; do it first.
2. Create a **WhatsApp Business Account (WABA)**.
3. Add a **phone number** dedicated to the WABA (must NOT be active on the
   regular WhatsApp app). A free **test number** is available immediately and can
   message up to 5 pre-registered recipients — use it to demo same-day while
   verification is pending.
4. In the app dashboard, note the **Phone number ID** and **WABA ID**.
5. Create a **System User** + permanent **access token** (Graph API).
6. Copy the **App secret** (App → Settings → Basic) — verifies inbound signatures.
7. Choose a **verify token** (any random string you pick) for the webhook GET handshake.

## 2. CO env vars

```bash
# Outbound (CloudApiProvider)
flyctl secrets set \
  WHATSAPP_CLOUD_TOKEN="<system-user-token>" \
  WHATSAPP_PHONE_NUMBER_ID="<phone-number-id>" \
  WHATSAPP_GRAPH_VERSION="v21.0" \
  -a co-artelonga

# Inbound webhook (whatsapp_cloud_routes)
flyctl secrets set \
  WHATSAPP_VERIFY_TOKEN="<the-string-you-picked>" \
  WHATSAPP_APP_SECRET="<meta-app-secret>" \
  -a co-artelonga
```

When `WHATSAPP_CLOUD_TOKEN` + `WHATSAPP_PHONE_NUMBER_ID` are set, CO's WhatsApp
sends (e.g. recovery codes) use the **Cloud API**; otherwise they fall back to
Evolution, then to logging.

## 3. Register the webhook with Meta

Endpoint (already wired in CO):

```
GET/POST  https://co.artelonga.com.br/api/v1/whatsapp/webhook
```

In the Meta app → WhatsApp → Configuration → Webhook:
- **Callback URL**: the URL above.
- **Verify token**: the same `WHATSAPP_VERIFY_TOKEN`.
- Subscribe to the **`messages`** field.

Meta sends `GET ...?hub.mode=subscribe&hub.verify_token=...&hub.challenge=...`;
CO echoes `hub.challenge` when the token matches. Inbound `POST`s are rejected
(`401`) unless `X-Hub-Signature-256` validates against `WHATSAPP_APP_SECRET`.

### Dev (no Fly): tunnel to localhost

Meta needs a public URL, so Tailscale won't work here (it's private). Use a tunnel:

```bash
cloudflared tunnel --url http://localhost:3940    # or: ngrok http 3940
# register the printed https URL + /api/v1/whatsapp/webhook with Meta
```

## 4. Compliance checklist (you have CNPJ + consent)

- [ ] Business verification approved (or using the test number for a demo).
- [ ] **Opt-in** recorded per recipient before any business-initiated message (LGPD).
- [ ] Message **templates** submitted + approved for anything outside the 24h
      customer-service window (free-form text only works inside the window).
- [ ] Display name approved.

## 5. Smoke test

```bash
# Outbound (inside a 24h window or to a test recipient)
curl -s "https://graph.facebook.com/v21.0/$WHATSAPP_PHONE_NUMBER_ID/messages" \
  -H "Authorization: Bearer $WHATSAPP_CLOUD_TOKEN" -H "Content-Type: application/json" \
  -d '{"messaging_product":"whatsapp","to":"<recipient>","type":"text","text":{"body":"oi do Co"}}'

# Inbound: send a WhatsApp message to the number → check CO logs
flyctl logs -a co-artelonga | grep "WhatsApp Cloud inbound"
```

## 6. Follow-ups

- **CO-480**: route inbound webhook → bot brain → auto-reply via `CloudApiProvider`
  (the brain currently lives in `~/projects/tools/whatsapp-bot`).
- Template registry + opt-in store in CO.
- BSP variant (Take Blip / Zenvia / 360dialog) — same trait, different base URL/auth.
```
