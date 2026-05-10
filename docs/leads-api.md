# Leads API — CO-183

Intake endpoint for contact form submissions from artelonga.com.br/contato/,
replacing the previous `mailto:` fallback. Leads are persisted in SQLite,
triaged via an admin queue, and optionally promoted to AL-tasks.

## Endpoints

### `POST /api/v1/leads` — public

Accepts a form submission. No authentication required.

**Body** (all fields except `mensagem` are optional):
```json
{
  "nome": "Maria Silva",
  "email": "maria@example.com",
  "telefone": "+5511999999999",
  "mensagem": "Preciso de assistência técnica...",
  "servico_titulo": "Assistência Técnica",
  "parceiro_handle": "matheus"
}
```

**Rules:**
- `mensagem` required, max 4 000 chars.
- Bot user-agents (crawlers, headless browsers) receive `200 OK` silently — lead is not stored.
- Rate limit: 5 submissions per IP-hash per 24 h → `429 Too Many Requests` on excess.
- Raw IP is **never** stored; only a daily-salted hash (`ip_hash`).
- User-agent stored raw, trimmed to 256 chars.
- An email notification is dispatched to `LEADS_NOTIFY_TO` (default: `rede@artelonga.com.br`) after every successful submission. Email delivery failure does **not** fail the POST.

**Responses:**
- `201 Created` — `{"id": N}` (lead ID only; no other details exposed).
- `400 Bad Request` — validation failure.
- `429 Too Many Requests` — rate limit exceeded.

**CORS:** same policy as the rest of the platform (mirrors `Origin` header).

---

### `GET /api/v1/admin/leads` — admin

List leads. Requires JWT from `Authorization: Bearer <token>` or `session` cookie,
plus `CO_SEED_ADMIN_EMAIL` match (same gate as `/api/v1/admin/dashboard`).

**Query parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `status` | string | Filter by status: `new`, `triaged`, `in_progress`, `closed` |
| `since` | date | ISO date (`YYYY-MM-DD`) — include leads on or after this date |
| `assignee` | string | Filter by `assignee_handle` |
| `limit` | integer | Max results (default 50, max 200) |

**Response:**
```json
{
  "leads": [
    {
      "id": 42,
      "created_at": "2026-05-10T12:00:00Z",
      "updated_at": "2026-05-10T12:00:00Z",
      "nome": "Maria Silva",
      "email": "maria@example.com",
      "telefone": "+5511999999999",
      "mensagem": "Preciso de assistência técnica...",
      "servico_titulo": "Assistência Técnica",
      "parceiro_handle": "matheus",
      "status": "new",
      "priority": "normal",
      "assignee_handle": null,
      "notes": null,
      "closed_reason": null,
      "promoted_to_al": null
    }
  ],
  "total": 42
}
```

- `total` reflects the full count matching the applied filters (before `limit`).
- Results ordered by `created_at DESC`.

**Errors:** `401 Unauthorized`, `403 Forbidden`.

---

### `PATCH /api/v1/admin/leads/:id` — admin

Partial update for triage. Same admin gate as GET above.

**Body** (all fields optional):
```json
{
  "status": "triaged",
  "priority": "high",
  "assignee_handle": "yuri",
  "notes": "Cliente quer site novo, budget 8k.",
  "closed_reason": null,
  "promoted_to_al": null
}
```

**Valid status values:** `new`, `triaged`, `in_progress`, `closed`.

**Valid priority values:** `low`, `normal`, `high`, `urgent`.

**Valid closed_reason values:** `won`, `lost`, `spam`, `duplicate`.

**State machine:**
```
new → triaged → in_progress → closed
new → in_progress (skip triaged)
```
Invalid transitions return `400 Bad Request`.

`updated_at` is automatically bumped on every successful PATCH.

**Responses:**
- `200 OK` — `{"ok": true}`.
- `400 Bad Request` — invalid status, priority, closed_reason, or invalid state transition.
- `404 Not Found` — lead ID does not exist.
- `401 / 403` — auth failure.

---

### `GET /admin/leads.html` — admin static page

Serves the leads admin SPA. Cookie auth (`session` JWT, same gate as `/admin`).
Redirects to `/` on missing/invalid session, returns `403` for non-admin users.

Features:
- Summary chips: new / triaged / in_progress / closed counts.
- Filterable table: status, since (date), assignee.
- Detail panel (click any row): full lead data + PATCH form.
- Auto-refreshes every 60 seconds.
- URL-anchored detail view (`#lead-N`).

---

## Schema

```sql
CREATE TABLE leads (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    nome            TEXT,
    email           TEXT,
    telefone        TEXT,
    mensagem        TEXT NOT NULL,
    servico_titulo  TEXT,
    parceiro_handle TEXT,
    status          TEXT NOT NULL DEFAULT 'new',
    priority        TEXT DEFAULT 'normal',
    assignee_handle TEXT,
    notes           TEXT,
    closed_reason   TEXT,
    promoted_to_al  INTEGER,
    ip_hash         TEXT,         -- daily-salted hash, raw IP never stored
    user_agent      TEXT          -- trimmed to 256 chars
);
```

Indexes: `idx_leads_status`, `idx_leads_created_at`, `idx_leads_assignee`.

---

## Email notification

On every successful `POST /api/v1/leads`, a notification email is sent:

- **To:** `LEADS_NOTIFY_TO` env var (default: `rede@artelonga.com.br`)
- **Subject:** `[Lead #N] {nome ?? "Anônimo"} via {servico_titulo ?? "form geral"}`
- **Body:** all lead fields + admin link to `https://co.artelonga.com.br/admin/leads.html#lead-N`

Delivery chain: Resend API → SMTP (`CO_SMTP_*` env vars) → log-only fallback.
Email failure never fails the lead POST — the lead is persisted regardless.

---

## Privacy / LGPD

- **Raw IP:** never stored. Only a daily-salted `xxh3` hash (`ip_hash`) is kept;
  the same IP produces a different hash the next day, preventing cross-day tracking.
- **Email opt-in:** submitting the contact form implies implicit consent per LGPD Art. 7(V).
  Opt-out: operator deletes the row via support request.
- **Retention:** closed leads older than 24 months are automatically purged by a
  daily background task (`retention_task`). Open/active leads are retained until
  manually closed.
- **Privacy notice:** the contact form at artelonga.com.br/contato/ must include
  a notice referencing this data collection (responsibility of AL-4 / frontend).

### Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `LEADS_NOTIFY_TO` | `rede@artelonga.com.br` | Email address that receives lead notifications |
| `RESEND_API_KEY` | — | Enables Resend delivery (preferred) |
| `RESEND_FROM` | `CO <noreply@quilomboaraucaria.com.br>` | Sender address for Resend |
| `CO_SMTP_HOST` / `CO_SMTP_USER` / `CO_SMTP_PASS` / `CO_SMTP_FROM` | — | SMTP fallback |

---

## Out of scope (future)

- Customer-facing status page (client views their own lead via token-link).
- SLA tracking and reminders.
- Webhook integrations (Slack, Discord).
- Lead → AL-task promotion CLI helper.
- Multi-tenant leads (for universes other than artelonga).
