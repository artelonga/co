/**
 * CO-124 — CO tail Worker
 *
 * Subscribes to a target Cloudflare Worker's log stream via the `tail()` handler.
 * Maps each log event to a CO TelemetryEvent, gzip-compresses the JSON-Lines
 * payload, signs it with HMAC-SHA256, and POSTs to the CO ingest endpoint.
 *
 * Wire format matches the Fly sidecar (CO-120):
 *   Content-Type: application/x-ndjson
 *   Content-Encoding: gzip          ← gzip instead of zstd (CF Workers limitation)
 *   X-Co-Timestamp: <unix_seconds>
 *   X-Co-Universe: <universe_id>
 *   X-Co-Signature: <hmac_sha256_hex>
 *
 * HMAC covers: "<timestamp>|<universe_id>|<hex(SHA256(compressed_body))>"
 *
 * Deploy:
 *   wrangler secret put CO_HMAC_KEY
 *   wrangler deploy
 *
 * Attach to your target Worker (in its wrangler.toml):
 *   [[tail_consumers]]
 *   service = "co-tail"
 */

export interface Env {
  /** CO ingest URL — e.g. https://co.artelonga.com.br/v1/ingest/events */
  CO_INGEST_URL: string;
  /** Universe slug this tail Worker reports for. */
  CO_UNIVERSE_ID: string;
  /** Hex-encoded 32-byte HMAC-SHA256 key. Set via `wrangler secret put CO_HMAC_KEY`. */
  CO_HMAC_KEY: string;
}

interface CoEvent {
  id: string;
  timestamp: string;
  universe_id: string;
  kind: "log";
  payload: {
    message: string;
    level: string;
    script_name: string;
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function hexEncode(buf: ArrayBuffer): string {
  return Array.from(new Uint8Array(buf))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

async function gzipCompress(data: Uint8Array): Promise<Uint8Array> {
  const stream = new CompressionStream("gzip");
  const writer = stream.writable.getWriter();
  await writer.write(data);
  await writer.close();
  const chunks: Uint8Array[] = [];
  const reader = stream.readable.getReader();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
  }
  const total = chunks.reduce((n, c) => n + c.length, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const c of chunks) {
    out.set(c, off);
    off += c.length;
  }
  return out;
}

async function buildHeaders(
  body: Uint8Array,
  universeId: string,
  keyHex: string,
): Promise<Headers> {
  const timestamp = Math.floor(Date.now() / 1000);
  const bodyHash = hexEncode(await crypto.subtle.digest("SHA-256", body));
  const message = `${timestamp}|${universeId}|${bodyHash}`;

  const key = await crypto.subtle.importKey(
    "raw",
    hexToBytes(keyHex),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = hexEncode(
    await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(message)),
  );

  const h = new Headers();
  h.set("Content-Type", "application/x-ndjson");
  h.set("Content-Encoding", "gzip");
  h.set("X-Co-Timestamp", String(timestamp));
  h.set("X-Co-Universe", universeId);
  h.set("X-Co-Signature", sig);
  return h;
}

function logEntryToEvent(log: TraceLog, scriptName: string, universeId: string): CoEvent {
  const rawMessage = log.message;
  const message = Array.isArray(rawMessage)
    ? rawMessage.join(" ")
    : String(rawMessage ?? "");
  return {
    id: crypto.randomUUID(),
    timestamp: new Date(log.timestamp ?? Date.now()).toISOString(),
    universe_id: universeId,
    kind: "log",
    payload: {
      message,
      level: log.level ?? "log",
      script_name: scriptName,
    },
  };
}

// ---------------------------------------------------------------------------
// Tail handler
// ---------------------------------------------------------------------------

export default {
  async tail(events: TraceItem[], env: Env): Promise<void> {
    const coEvents: CoEvent[] = events.flatMap((e) =>
      (e.logs ?? []).map((log) =>
        logEntryToEvent(log, e.scriptName ?? "", env.CO_UNIVERSE_ID),
      ),
    );

    if (coEvents.length === 0) return;

    const ndjson = new TextEncoder().encode(
      coEvents.map((e) => JSON.stringify(e)).join("\n"),
    );
    const body = await gzipCompress(ndjson);
    const headers = await buildHeaders(body, env.CO_UNIVERSE_ID, env.CO_HMAC_KEY);

    const resp = await fetch(env.CO_INGEST_URL, { method: "POST", headers, body });
    if (!resp.ok) {
      console.error(`co-tail: ingest returned ${resp.status}`);
    }
  },
} satisfies ExportedHandler<Env>;
