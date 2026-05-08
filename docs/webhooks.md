# CO Outbound Webhooks

CO emits signed HTTP POST events to registered endpoints. Use this to connect CO and quilombo events to n8n, Zapier, or any automation platform — without writing Rust code.

## How it works

```
Request handler
  └── emit_event("quilombo.evento.criado", payload)
        └── notifications table (pending)
              └── webhook_worker (background, polls every 5 s)
                    ├── claim row
                    ├── POST {url} with HMAC-SHA256 signature
                    ├── mark sent / failed
                    └── retry up to 3× with 5 s / 30 s / 2 min backoff
```

Each delivery includes these headers:

| Header | Value |
|--------|-------|
| `X-CO-Event` | event type, e.g. `quilombo.evento.criado` |
| `X-CO-Delivery` | unique notification ID (UUID) |
| `X-CO-Signature-256` | `sha256=<hmac_hex>` |
| `Content-Type` | `application/json` |

The HMAC-SHA256 signature covers the raw request body using the webhook's secret.
This matches GitHub's webhook signature scheme, so existing n8n/Zapier GitHub nodes
work with minimal adaptation.

## Event catalogue

| Event type | Trigger |
|------------|---------|
| `quilombo.evento.criado` | New event created |
| `quilombo.missao.participou` | User joined a mission |
| `quilombo.mensagem.criada` | New internal message sent |
| `quilombo.usuario.cadastro` | New quilombo user registered |
| `quilombo.usuario.login` | User logged in |
| `quilombo.missao.criada` | New mission created |
| `co.universe.criado` | New universe created |
| `co.entry.criado` | New entry created (opt-in; high volume) |
| `co.usuario.cadastro` | New CO user registered |

### Wildcard patterns

When registering a webhook, the `events` field accepts wildcards:

| Pattern | Matches |
|---------|---------|
| `*` | All events |
| `quilombo.*` | All quilombo events |
| `co.*` | All CO platform events |
| `quilombo.evento.criado` | Exact match only |

## Admin API

All endpoints require a GitHub PAT from an allowed admin in the `Authorization: Bearer <token>` header.

### Register a webhook

```
POST /api/v1/gestao/webhooks
Content-Type: application/json

{
  "url": "https://n8n.example.com/webhook/co",
  "events": ["quilombo.*"]
}
```

Response (201 Created) — **secret is shown only once**:

```json
{
  "id": "abc123",
  "url": "https://n8n.example.com/webhook/co",
  "secret": "a8f3d...",
  "events": ["quilombo.*"],
  "enabled": true,
  "created_at": "2026-05-08T12:00:00Z"
}
```

### List webhooks

```
GET /api/v1/gestao/webhooks
```

Returns all webhooks with `secret` omitted.

### Update a webhook

```
PUT /api/v1/gestao/webhooks/:id
Content-Type: application/json

{
  "enabled": false
}
```

All fields (`url`, `events`, `enabled`) are optional.

### Delete a webhook

```
DELETE /api/v1/gestao/webhooks/:id
```

Deletes the webhook and cascades to all pending/sent notifications.

### Delivery log

```
GET /api/v1/gestao/webhooks/:id/deliveries
```

Returns the last 100 notification rows (newest first) with status, attempts, and error details.

## Retry policy

| Attempt | Delay before retry |
|---------|--------------------|
| 1st failure | 5 seconds |
| 2nd failure | 30 seconds |
| 3rd failure | 2 minutes |
| 4th failure | marked `dead` (no more retries) |

2xx HTTP responses mark the delivery `sent`. Any other status or network error triggers a retry.

## n8n integration

### 1. Create a Webhook node in n8n

- Method: **POST**
- URL: copy the n8n webhook URL (e.g. `https://n8n.example.com/webhook/co`)

### 2. Register the n8n URL in CO

```bash
curl -X POST https://co.artelonga.com.br/api/v1/gestao/webhooks \
  -H "Authorization: Bearer $GITHUB_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "url": "https://n8n.example.com/webhook/co",
    "events": ["quilombo.*"]
  }'
```

Store the returned `secret` — it will not be shown again.

### 3. Validate the signature in n8n

Add a **Code** node before your routing logic:

```javascript
const crypto = require('crypto');

const secret = $env.CO_WEBHOOK_SECRET; // set in n8n credentials
const body = $input.first().binary?.data?.toString() ?? JSON.stringify($input.first().json);
const signature = $input.first().headers['x-co-signature-256'];

const expected = 'sha256=' + crypto
  .createHmac('sha256', secret)
  .update(body)
  .digest('hex');

if (signature !== expected) {
  throw new Error('Invalid signature');
}

return $input.all();
```

### 4. Route by event type

Add a **Switch** node after signature validation, branching on `x-co-event`:

| Condition | Downstream |
|-----------|-----------|
| `quilombo.evento.criado` | WhatsApp node |
| `quilombo.mensagem.criada` | Email node |
| `quilombo.missao.participou` | SMS node |

## Zapier integration

1. Create a **Webhooks by Zapier** trigger (Catch Hook)
2. Register the Zapier URL via the Admin API
3. In the Zap, add a **Code** step to verify `X-CO-Signature-256` using the same HMAC-SHA256 scheme as above
4. Route by `X-CO-Event` header to subsequent Zap steps

## Security notes

- Never expose the webhook secret. If compromised, delete the webhook and create a new one.
- Always validate the signature before acting on a delivery.
- Deliveries time out after 10 seconds — keep your webhook handler fast.
- The worker polls every 5 seconds — expect up to 5 s of latency between the event and delivery.
