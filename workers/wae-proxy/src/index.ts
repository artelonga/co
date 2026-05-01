/**
 * CO-118 — WAE Worker proxy
 *
 * Accepts POST /api/internal/wae from co-web (Bearer token auth),
 * runs the privacy filter, then writes a data point to the
 * `co_telemetry` Analytics Engine dataset via env.WAE.writeDataPoint().
 *
 * Deploy:
 *   wrangler secret put WAE_API_KEY
 *   wrangler deploy
 *
 * Query:
 *   curl "https://api.cloudflare.com/client/v4/accounts/<ACCOUNT_ID>/analytics_engine/sql" \
 *     -H "Authorization: Bearer <CF_API_TOKEN>" \
 *     --data "SELECT * FROM co_telemetry LIMIT 10"
 */

export interface Env {
  WAE: AnalyticsEngineDataset;
  WAE_API_KEY: string;
}

// ---------------------------------------------------------------------------
// Privacy filter — mirrors co-web/src/wae.rs
// ---------------------------------------------------------------------------

const MAX_FIELD_LEN = 256;
const BLOCKED_KEY_PATTERNS = ["content", "body", "text", "markdown", "prose"];

interface PrivacyError {
  field: string;
  reason: string;
}

function checkField(key: string, value: string): PrivacyError | null {
  const keyLc = key.toLowerCase();
  for (const pattern of BLOCKED_KEY_PATTERNS) {
    if (keyLc.includes(pattern)) {
      return { field: key, reason: `field name matches blocked pattern '${pattern}'` };
    }
  }
  if (value.length > MAX_FIELD_LEN) {
    return { field: key, reason: `value exceeds ${MAX_FIELD_LEN} bytes` };
  }
  return null;
}

interface TelemetryEvent {
  event_type: string;
  universe_id: string;
  user_kind: string;
  flag_key?: string;
  variant?: string;
  duration_ms?: number;
  status?: number;
}

function validateEvent(ev: unknown): { ok: true; event: TelemetryEvent } | { ok: false; error: PrivacyError } {
  if (typeof ev !== "object" || ev === null) {
    return { ok: false, error: { field: "_root", reason: "payload must be a JSON object" } };
  }

  const obj = ev as Record<string, unknown>;

  // Check all string fields for blocked names and length
  for (const [key, val] of Object.entries(obj)) {
    if (typeof val === "string") {
      const err = checkField(key, val);
      if (err) return { ok: false, error: err };
    }
  }

  // Required fields
  const event_type = typeof obj.event_type === "string" ? obj.event_type : "";
  const universe_id = typeof obj.universe_id === "string" ? obj.universe_id : "";
  const user_kind = typeof obj.user_kind === "string" ? obj.user_kind : "";

  if (!event_type || !universe_id || !user_kind) {
    return {
      ok: false,
      error: { field: "event_type|universe_id|user_kind", reason: "required fields missing or empty" },
    };
  }

  return {
    ok: true,
    event: {
      event_type,
      universe_id,
      user_kind,
      flag_key: typeof obj.flag_key === "string" ? obj.flag_key : undefined,
      variant: typeof obj.variant === "string" ? obj.variant : undefined,
      duration_ms: typeof obj.duration_ms === "number" ? obj.duration_ms : undefined,
      status: typeof obj.status === "number" ? obj.status : undefined,
    },
  };
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    // Only POST /api/internal/wae
    const url = new URL(request.url);
    if (request.method !== "POST" || url.pathname !== "/api/internal/wae") {
      return new Response("Not Found", { status: 404 });
    }

    // Bearer token auth
    const auth = request.headers.get("Authorization") ?? "";
    if (!auth.startsWith("Bearer ") || auth.slice(7) !== env.WAE_API_KEY) {
      return new Response(JSON.stringify({ error: "Unauthorized" }), {
        status: 401,
        headers: { "Content-Type": "application/json" },
      });
    }

    // Parse body
    let body: unknown;
    try {
      body = await request.json();
    } catch {
      return new Response(JSON.stringify({ error: "invalid JSON" }), {
        status: 400,
        headers: { "Content-Type": "application/json" },
      });
    }

    // Privacy filter
    const result = validateEvent(body);
    if (!result.ok) {
      return new Response(JSON.stringify({ error: "privacy_filter", detail: result.error }), {
        status: 422,
        headers: { "Content-Type": "application/json" },
      });
    }

    const ev = result.event;

    // Write to WAE dataset `co_telemetry`
    // Layout:
    //   indexes[0] = universe_id   (partition key — indexed for fast WHERE)
    //   blobs[0]   = event_type
    //   blobs[1]   = user_kind
    //   blobs[2]   = flag_key      (empty string when absent)
    //   blobs[3]   = variant       (empty string when absent)
    //   doubles[0] = duration_ms   (0 when absent)
    //   doubles[1] = status        (0 when absent)
    env.WAE.writeDataPoint({
      indexes: [ev.universe_id],
      blobs: [ev.event_type, ev.user_kind, ev.flag_key ?? "", ev.variant ?? ""],
      doubles: [ev.duration_ms ?? 0, ev.status ?? 0],
    });

    return new Response(null, { status: 204 });
  },
};
