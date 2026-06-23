/**
 * CO-375 — Contract probe: hit every declared endpoint against staging,
 * detect status-code drift and response-shape drift.
 *
 * Usage (CI):
 *   STAGING_URL=https://co-artelonga-staging.fly.dev \
 *   CO_STAGING_ADMIN_TOKEN=<bearer-token> \
 *   node --import tsx co-web/scripts/contract/probe-staging.ts
 *
 * Outputs:
 *   - contract-probe-report.json  (machine-readable, current directory)
 *   - stdout summary              (human-readable)
 *
 * Exit codes:
 *   0  — no drift detected
 *   1  — drift detected or probe error
 */

import { readFileSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname_script = dirname(fileURLToPath(import.meta.url));
// co-web/scripts/contract → co-web → repo root
const OPENAPI_PATH = join(__dirname_script, '../../openapi.yaml');

// ─── Config ──────────────────────────────────────────────────────────────────

const STAGING_URL = (process.env.STAGING_URL ?? process.env.PROBE_URL ?? 'https://co.artelonga.com.br').replace(/\/$/, '');
const ADMIN_TOKEN = process.env.CO_STAGING_ADMIN_TOKEN ?? '';
const REPORT_PATH = process.env.REPORT_PATH ?? 'contract-probe-report.json';
const TIMEOUT_MS = Number(process.env.PROBE_TIMEOUT_MS ?? 20_000);
const CONCURRENCY = Number(process.env.PROBE_CONCURRENCY ?? 6);

// ─── Types ───────────────────────────────────────────────────────────────────

interface OpenApiSpec {
  paths: Record<string, Record<string, OpenApiOperation>>;
}

interface OpenApiOperation {
  summary?: string;
  tags?: string[];
  security?: Array<Record<string, string[]>>;
  parameters?: Array<{ name: string; in: string; required?: boolean }>;
  responses?: Record<string, { description?: string }>;
}

type DriftType = 'status_code' | 'shape' | 'server_error' | 'route_missing';

interface ProbeResult {
  endpoint: string;
  url: string;
  method: string;
  path: string;
  resolved_url: string;
  declared_status: number;
  actual_status: number | null;
  response_time_ms: number;
  drift: boolean;
  drift_type?: DriftType;
  drift_detail?: string;
  skipped: false;
}

interface SkippedResult {
  endpoint: string;
  path: string;
  method: string;
  skip_reason: string;
  skipped: true;
}

type Result = ProbeResult | SkippedResult;

interface Report {
  timestamp: string;
  staging_url: string;
  openapi_path: string;
  summary: {
    total: number;
    probed: number;
    skipped: number;
    passed: number;
    failed: number;
  };
  drift: ProbeResult[];
  passed: ProbeResult[];
  skipped: SkippedResult[];
}

// ─── Skip rules ──────────────────────────────────────────────────────────────

// Endpoints that issue tokens / start async flows / have destructive side effects.
// Probing these would break staging data or trigger external services.
const SKIP_ENDPOINT_KEYS = new Set([
  // Config-dependent: handler 404s by design when VAPID_PUBLIC_KEY secret is
  // absent (staging has no push credentials) — indistinguishable from a
  // removed route from outside.
  'GET /api/v1/notifications/vapid-public-key',
  // Token issuance — require fresh user interaction
  'POST /api/v1/auth/token',
  'POST /api/v1/auth/tokens',
  'DELETE /api/v1/auth/tokens/{id}',
  'POST /api/v1/auth/login',
  'POST /api/v1/auth/verify',
  'POST /api/v1/auth/signup',
  'POST /api/v1/auth/register',
  'POST /api/v1/auth/onboard-with-email',
  'POST /api/v1/auth/onboard-with-email/verify',
  'POST /api/v1/auth/legacy-login',
  'POST /api/v1/auth/uat-login',
  'POST /api/v1/auth/password-login',
  'POST /api/v1/auth/change-password',
  'POST /api/v1/auth/forgot-password',
  'POST /api/v1/auth/forgot-password/verify',
  'POST /api/v1/auth/reset-password',
  'POST /api/v1/auth/recovery/channels',
  'POST /api/v1/auth/recovery/channels/verify',
  'DELETE /api/v1/auth/recovery/channels/{id}',
  // OAuth flows — browser redirect required
  'GET /api/v1/auth/google/start',
  'GET /api/v1/auth/google/callback',
  'POST /api/v1/auth/google/callback',
  'GET /auth/co-handover',
  'GET /oauth/authorize',
  'POST /oauth/token',
  // Destructive / mass-mutating operations
  'POST /api/v1/universes',
  'POST /api/v1/universes/apply-template-all',
  'POST /api/v1/universes/{slug}/apply-template',
  'POST /api/v1/universes/{slug}/clone',
  'POST /api/v1/universes/{slug}/duplicate',
  'POST /api/v1/universes/{slug}/promote',
  'DELETE /api/v1/universes/{slug}',
  'POST /api/v1/universes/{slug}/reindex',
  'POST /api/v1/admin/backup/snapshot',
  'POST /api/v1/admin/changelog/reindex',
  // Streaming (SSE/WS) — not probed here
  'GET /api/v1/events',
  'POST /api/v1/events',
  'GET /api/v1/events/bridge',
  'POST /api/v1/events/bridge',
  // UAT-only endpoints
  'GET /api/v1/uat/changes',
  'GET /api/v1/uat/export-patch',
  // Vault token endpoints — require fresh API token issuance
  // (vault read endpoints use sessionCookie and are probed normally)
]);

// Path parameter values that can be resolved without staging state knowledge.
// Params not in this map cause the endpoint to be skipped (unparameterized).
const PARAM_VALUES: Record<string, string> = {
  slug: 'template',      // universe slug — always exists on staging
  universe: 'template',
  u: 'template',
  preset: 'modern',      // theme preset
  game_name: 'example',  // placeholder game name
};

// ─── Known response schemas ───────────────────────────────────────────────────
// The openapi.yaml response sections carry only `description: OK` (no inline
// schemas). We embed contracts here for key endpoints so ajv can validate shape.

interface JsonSchema {
  type?: string;
  required?: string[];
  properties?: Record<string, JsonSchema>;
  items?: JsonSchema;
  additionalProperties?: boolean | JsonSchema;
  enum?: unknown[];
  pattern?: string;
  minimum?: number;
}

const INLINE_SCHEMAS: Record<string, JsonSchema> = {
  'GET /api/health': {
    type: 'object',
    required: ['status', 'version'],
    properties: {
      status: { type: 'string' },
      version: { type: 'string' },
    },
    additionalProperties: true,
  },
  'GET /api/health/deep': {
    type: 'object',
    required: ['status'],
    properties: {
      status: { type: 'string' },
    },
    additionalProperties: true,
  },
  'GET /.well-known/jwks.json': {
    type: 'object',
    required: ['keys'],
    properties: {
      keys: { type: 'array', items: { type: 'object' } },
    },
    additionalProperties: true,
  },
  'GET /api/openapi.json': {
    type: 'object',
    required: ['openapi', 'paths'],
    properties: {
      openapi: { type: 'string' },
      paths: { type: 'object' },
    },
    additionalProperties: true,
  },
  'GET /api/v1/auth/me': {
    type: 'object',
    required: ['user_id'],
    properties: {
      user_id: { type: 'string' },
    },
    additionalProperties: true,
  },
  'GET /api/v1/universes/public': {
    type: 'array',
    items: {
      type: 'object',
      required: ['key'],
      properties: { key: { type: 'string' } },
      additionalProperties: true,
    },
  },
};

// ─── YAML parser (minimal — handles the openapi.yaml structure we generate) ──
// Rather than pulling in a full yaml dep, we parse paths from the raw YAML
// using a line-by-line state machine. The generated openapi.yaml has a
// predictable structure from generate-openapi.ts so this is safe.

function parseOpenApiPaths(yaml: string): OpenApiSpec['paths'] {
  const paths: OpenApiSpec['paths'] = {};
  const lines = yaml.split('\n');
  let inPaths = false;
  let currentPath: string | null = null;
  let currentMethod: string | null = null;
  let inSecurity = false;
  let inParameters = false;
  let inResponses = false;
  let securityEntries: Array<Record<string, string[]>> = [];

  const HTTP_METHODS = new Set(['get', 'post', 'put', 'patch', 'delete', 'head', 'options']);

  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const trimmed = raw.trim();

    // Detect `paths:` section
    if (raw === 'paths:') {
      inPaths = true;
      continue;
    }
    if (!inPaths) continue;

    // Stop at `components:` or another top-level section
    if (/^[a-z]/.test(raw) && !raw.startsWith('paths:') && !raw.startsWith('  ')) {
      inPaths = false;
      continue;
    }

    // Path key: two-space indent + path
    const pathMatch = raw.match(/^  (\/[^\s:]+):$/);
    if (pathMatch) {
      currentPath = pathMatch[1];
      currentMethod = null;
      inSecurity = false;
      inParameters = false;
      inResponses = false;
      paths[currentPath] = {};
      continue;
    }

    if (!currentPath) continue;

    // Method key: four-space indent
    const methodMatch = raw.match(/^    ([a-z]+):$/);
    if (methodMatch && HTTP_METHODS.has(methodMatch[1])) {
      currentMethod = methodMatch[1];
      inSecurity = false;
      inParameters = false;
      inResponses = false;
      securityEntries = [];
      paths[currentPath][currentMethod] = { responses: { '200': { description: 'OK' } } };
      continue;
    }

    if (!currentMethod) continue;

    const op = paths[currentPath][currentMethod];

    // Summary
    if (trimmed.startsWith('summary:')) {
      op.summary = trimmed.replace(/^summary:\s*"?/, '').replace(/"$/, '');
      continue;
    }

    // Tags
    if (trimmed.startsWith('tags:')) {
      const m = trimmed.match(/\["([^"]+)"\]/);
      if (m) op.tags = [m[1]];
      continue;
    }

    // Security block
    if (trimmed === 'security:') {
      inSecurity = true;
      inParameters = false;
      inResponses = false;
      continue;
    }
    if (inSecurity && trimmed.startsWith('- ')) {
      const key = trimmed.slice(2).replace(/:.*$/, '');
      securityEntries.push({ [key]: [] });
      op.security = securityEntries;
      continue;
    }

    // Detect parameters/responses sections (stop security parsing)
    if (trimmed === 'parameters:') {
      inSecurity = false;
      inParameters = true;
      inResponses = false;
      continue;
    }
    if (trimmed === 'responses:') {
      inSecurity = false;
      inParameters = false;
      inResponses = true;
      continue;
    }

    // End of section when indent drops or new section starts
    if (inSecurity && raw.match(/^    [a-z]/)) {
      inSecurity = false;
    }
  }

  return paths;
}

// ─── Minimal JSON schema validator ───────────────────────────────────────────
// Validates a parsed JSON value against a subset of JSON Schema.
// Covers: type, required, properties, items, additionalProperties, enum.

function validateSchema(value: unknown, schema: JsonSchema, path = ''): string[] {
  const errors: string[] = [];

  if (schema.type) {
    const actual = Array.isArray(value) ? 'array' : typeof value;
    if (actual !== schema.type) {
      errors.push(`${path || 'root'}: expected ${schema.type}, got ${actual}`);
      return errors; // type mismatch → further checks won't be meaningful
    }
  }

  if (schema.type === 'object' && typeof value === 'object' && value !== null && !Array.isArray(value)) {
    const obj = value as Record<string, unknown>;

    if (schema.required) {
      for (const key of schema.required) {
        if (!(key in obj)) {
          errors.push(`${path || 'root'}: missing required field "${key}"`);
        }
      }
    }

    if (schema.properties) {
      for (const [key, propSchema] of Object.entries(schema.properties)) {
        if (key in obj) {
          errors.push(...validateSchema(obj[key], propSchema, `${path}.${key}`));
        }
      }
    }
  }

  if (schema.type === 'array' && Array.isArray(value) && schema.items && value.length > 0) {
    // Validate first element as representative sample
    errors.push(...validateSchema(value[0], schema.items, `${path}[0]`));
  }

  return errors;
}

// ─── Path param resolution ────────────────────────────────────────────────────

function resolvePathParams(path: string): { resolved: string; unresolved: string[] } {
  const unresolved: string[] = [];
  const resolved = path.replace(/\{([^}*]+)\}/g, (_, param) => {
    if (param in PARAM_VALUES) return PARAM_VALUES[param];
    unresolved.push(param);
    return `{${param}}`;
  }).replace(/\{\*([^}]+)\}/g, (_, param) => {
    // Wildcard params like {*path}
    const key = `*${param}`;
    if (key in PARAM_VALUES) return PARAM_VALUES[key];
    unresolved.push(`*${param}`);
    return `{*${param}}`;
  });
  return { resolved, unresolved };
}

// ─── Probe execution ──────────────────────────────────────────────────────────

async function probe(
  method: string,
  url: string,
  requiresAuth: boolean,
): Promise<{ status: number; body: unknown; contentType: string }> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);

  const headers: Record<string, string> = {
    Accept: 'application/json',
  };
  if (requiresAuth && ADMIN_TOKEN) {
    headers['Authorization'] = `Bearer ${ADMIN_TOKEN}`;
  }
  if (method !== 'GET' && method !== 'HEAD') {
    headers['Content-Type'] = 'application/json';
  }

  try {
    const resp = await fetch(url, {
      method,
      headers,
      body: method !== 'GET' && method !== 'HEAD' && method !== 'DELETE' ? '{}' : undefined,
      signal: controller.signal,
      redirect: 'manual', // don't follow redirects — 3xx is a valid response
    });

    clearTimeout(timer);

    const ct = resp.headers.get('content-type') ?? '';
    let body: unknown = null;
    if (ct.includes('application/json')) {
      try { body = await resp.json(); } catch { /* ignore parse error */ }
    } else {
      await resp.text(); // consume body
    }

    return { status: resp.status, body, contentType: ct };
  } catch (err: unknown) {
    clearTimeout(timer);
    const message = err instanceof Error ? err.message : String(err);
    throw new Error(`Network error: ${message}`);
  }
}

// ─── Drift classification ─────────────────────────────────────────────────────

// Optionally-configured features return a specific non-2xx in prod *by design* —
// this is expected, not drift. `auth/github/start` returns 503 when GitHub OAuth
// is intentionally not configured (prod runs without it). Documented-benign; a
// DIFFERENT status (or a different route drifting) still fails the probe.
const OPTIONAL_FEATURE_STATUS: Record<string, number[]> = {
  'GET /api/v1/auth/github/start': [503],
};

function classifyDrift(
  method: string,
  endpointKey: string,
  declaredStatus: number,
  actual: number,
  requiresAuth: boolean,
  hasToken: boolean,
  body: unknown,
): { drift: boolean; drift_type?: DriftType; detail?: string } {
  // Optionally-configured feature returning its expected non-2xx: not drift.
  const optionalOk = OPTIONAL_FEATURE_STATUS[endpointKey];
  if (optionalOk && optionalOk.includes(actual)) {
    return { drift: false };
  }

  // Server error is always drift — the endpoint crashed
  if (actual >= 500) {
    return { drift: true, drift_type: 'server_error', detail: `HTTP ${actual} (server error)` };
  }

  // 404 for a declared route = route has been removed — EXCEPT for routes
  // with path params: we probe them with the literal placeholder (e.g.
  // /paginas/{slug}), so the handler runs and correctly returns entity-404.
  // Router-404 and entity-404 are indistinguishable from outside; accept it.
  if (actual === 404) {
    if (endpointKey.includes('{')) {
      return { drift: false };
    }
    return { drift: true, drift_type: 'route_missing', detail: 'HTTP 404 — route not found (was it removed?)' };
  }

  // Auth-required endpoint probed without token: 401/403 is correct
  if (requiresAuth && !hasToken && (actual === 401 || actual === 403)) {
    return { drift: false };
  }

  // POST/PUT/PATCH with empty body: 400/422/415 is acceptable (validation error, not drift)
  if (method !== 'GET' && method !== 'HEAD' && method !== 'DELETE' && (actual === 400 || actual === 422 || actual === 415 || actual === 405)) {
    return { drift: false };
  }

  // DELETE on a non-existent resource: 404 would already be caught above;
  // 405 (method not allowed) = route exists but method wrong = drift
  if (actual === 405) {
    return { drift: true, drift_type: 'status_code', detail: `HTTP 405 — method not allowed (was method removed or changed?)` };
  }

  // GET probed without required query params: 400/422 proves the route
  // exists and validated input — not drift.
  if (method === 'GET' && (actual === 400 || actual === 422)) {
    return { drift: false };
  }

  // GET should return 200 on success (anon or authed)
  if (method === 'GET' && actual !== declaredStatus) {
    // 401/403 on an anon endpoint = auth was added (drift)
    if (!requiresAuth && (actual === 401 || actual === 403)) {
      return { drift: true, drift_type: 'status_code', detail: `HTTP ${actual} — endpoint now requires auth (declared: anon)` };
    }
    // 401/403 when we provided a valid token = token not accepted (drift)
    if (requiresAuth && hasToken && (actual === 401 || actual === 403)) {
      return { drift: true, drift_type: 'status_code', detail: `HTTP ${actual} — valid token rejected (auth check bug?)` };
    }
    // 3xx redirect on a GET (not expected for API endpoints)
    if (actual >= 300 && actual < 400) {
      return { drift: false }; // redirects are non-fatal (e.g. SPA routes)
    }
    // Anything else unexpected
    if (actual !== declaredStatus) {
      return { drift: true, drift_type: 'status_code', detail: `HTTP ${actual} (declared: ${declaredStatus})` };
    }
  }

  // Response shape validation for known schemas
  const schema = INLINE_SCHEMAS[endpointKey];
  if (schema && actual === 200 && body !== null) {
    const errors = validateSchema(body, schema);
    if (errors.length > 0) {
      return {
        drift: true,
        drift_type: 'shape',
        detail: `Schema mismatch: ${errors.join('; ')}`,
      };
    }
  }

  return { drift: false };
}

// ─── Concurrency pool ─────────────────────────────────────────────────────────

async function runConcurrent<T>(
  tasks: Array<() => Promise<T>>,
  concurrency: number,
): Promise<T[]> {
  const results: T[] = new Array(tasks.length);
  let idx = 0;

  async function worker() {
    while (idx < tasks.length) {
      const i = idx++;
      results[i] = await tasks[i]();
    }
  }

  const workers = Array.from({ length: Math.min(concurrency, tasks.length) }, worker);
  await Promise.all(workers);
  return results;
}

// ─── Main ─────────────────────────────────────────────────────────────────────

async function main() {
  const yamlText = readFileSync(OPENAPI_PATH, 'utf8');
  const paths = parseOpenApiPaths(yamlText);
  const hasToken = ADMIN_TOKEN.length > 0;

  if (!hasToken) {
    console.warn('[probe] Warning: CO_STAGING_ADMIN_TOKEN not set — skipping auth-required endpoints');
  }

  const HTTP_METHODS_PROBE = ['get', 'post', 'put', 'patch', 'delete', 'head'];

  // Build the full probe plan
  const results: Result[] = [];
  const tasks: Array<() => Promise<ProbeResult | null>> = [];

  for (const [pathTemplate, methods] of Object.entries(paths)) {
    for (const [methodLow, op] of Object.entries(methods)) {
      if (!HTTP_METHODS_PROBE.includes(methodLow)) continue;

      const method = methodLow.toUpperCase();
      const endpointKey = `${method} ${pathTemplate}`;
      const declaredStatus = 200;

      // 1. Skip list check
      if (SKIP_ENDPOINT_KEYS.has(endpointKey)) {
        results.push({
          endpoint: endpointKey,
          path: pathTemplate,
          method,
          skip_reason: 'token-issuance or destructive endpoint',
          skipped: true,
        });
        continue;
      }

      // 2. Resolve path parameters
      const { resolved, unresolved } = resolvePathParams(pathTemplate);
      if (unresolved.length > 0) {
        results.push({
          endpoint: endpointKey,
          path: pathTemplate,
          method,
          skip_reason: `unresolved path params: ${unresolved.join(', ')}`,
          skipped: true,
        });
        continue;
      }

      // 3. Determine auth requirement
      const requiresAuth = Array.isArray(op.security) && op.security.length > 0;

      // 4. Skip auth-required endpoints when no token is available
      if (requiresAuth && !hasToken) {
        results.push({
          endpoint: endpointKey,
          path: pathTemplate,
          method,
          skip_reason: 'auth required, CO_STAGING_ADMIN_TOKEN not set',
          skipped: true,
        });
        continue;
      }

      const url = `${STAGING_URL}${resolved}`;
      const resultIndex = results.length;
      results.push(null as unknown as Result); // placeholder

      tasks.push(async () => {
        const t0 = Date.now();
        let actualStatus: number | null = null;
        let body: unknown = null;
        let driftInfo: ReturnType<typeof classifyDrift> = { drift: false };
        let errorDetail: string | undefined;

        try {
          const r = await probe(method, url, requiresAuth);
          actualStatus = r.status;
          body = r.body;
          driftInfo = classifyDrift(method, endpointKey, declaredStatus, actualStatus, requiresAuth, hasToken, body);
        } catch (err: unknown) {
          errorDetail = err instanceof Error ? err.message : String(err);
          // A network abort / timeout / connection failure is an ENVIRONMENT
          // problem (target asleep, slow, or unreachable) — NOT a contract drift.
          // Treating it as drift fails the gate with an opaque exit 1 (the
          // /robots.txt "This operation was aborted" case). Retry once; if still
          // unreachable, do NOT count it as drift.
          if (/abort|timeout|network|ECONN|ENOTFOUND|EAI_AGAIN|fetch failed|socket/i.test(errorDetail)) {
            try {
              const r2 = await probe(method, url, requiresAuth);
              actualStatus = r2.status;
              body = r2.body;
              driftInfo = classifyDrift(method, endpointKey, declaredStatus, actualStatus, requiresAuth, hasToken, body);
            } catch (err2: unknown) {
              errorDetail = err2 instanceof Error ? err2.message : String(err2);
              driftInfo = { drift: false };
              console.warn(`[probe] unreachable (not drift): ${method} ${endpointKey} — ${errorDetail}`);
            }
          } else {
            driftInfo = { drift: true, drift_type: 'server_error', detail: errorDetail };
          }
        }

        const result: ProbeResult = {
          endpoint: endpointKey,
          url,
          method,
          path: pathTemplate,
          resolved_url: `${STAGING_URL}${resolved}`,
          declared_status: declaredStatus,
          actual_status: actualStatus,
          response_time_ms: Date.now() - t0,
          drift: driftInfo.drift,
          drift_type: driftInfo.drift_type,
          drift_detail: driftInfo.detail ?? errorDetail,
          skipped: false,
        };

        return result;
      });

      // Store index so we can fill placeholder
      const capturedIdx = resultIndex;
      const originalTask = tasks[tasks.length - 1];
      tasks[tasks.length - 1] = async () => {
        const r = await originalTask();
        results[capturedIdx] = r as Result;
        return r;
      };
    }
  }

  // Run probes concurrently
  console.log(`[probe] Staging: ${STAGING_URL}`);
  console.log(`[probe] Endpoints total:  ${results.length}`);
  const skippedCount = results.filter(r => r && 'skipped' in r && r.skipped).length;
  console.log(`[probe] Probing:          ${tasks.length}  (${skippedCount} skipped)`);

  await runConcurrent(tasks, CONCURRENCY);

  // Collect final results (remove nulls from placeholder slots)
  const allResults = results.filter(Boolean) as Result[];
  const probed = allResults.filter((r): r is ProbeResult => !r.skipped);
  const skipped = allResults.filter((r): r is SkippedResult => r.skipped);
  const drifted = probed.filter(r => r.drift);
  const passed = probed.filter(r => !r.drift);

  // ─── Human-readable summary ───────────────────────────────────────────────

  if (drifted.length > 0) {
    console.error('\n[probe] DRIFT DETECTED:\n');
    for (const d of drifted) {
      console.error(`  ✗ ${d.endpoint}`);
      console.error(`      ${d.drift_detail ?? `HTTP ${d.actual_status}`}`);
    }
  }

  console.log(`\n[probe] Summary`);
  console.log(`  Total declared:  ${allResults.length}`);
  console.log(`  Probed:          ${probed.length}`);
  console.log(`  Skipped:         ${skipped.length}`);
  console.log(`  Passed:          ${passed.length}`);
  console.log(`  Drift detected:  ${drifted.length}`);

  if (drifted.length === 0) {
    console.log('\n[probe] ✓ No drift — staging contract matches openapi.yaml');
  }

  // ─── Machine-readable report ──────────────────────────────────────────────

  const report: Report = {
    timestamp: new Date().toISOString(),
    staging_url: STAGING_URL,
    openapi_path: OPENAPI_PATH,
    summary: {
      total: allResults.length,
      probed: probed.length,
      skipped: skipped.length,
      passed: passed.length,
      failed: drifted.length,
    },
    drift: drifted,
    passed,
    skipped,
  };

  writeFileSync(REPORT_PATH, JSON.stringify(report, null, 2), 'utf8');
  console.log(`\n[probe] Report written to: ${REPORT_PATH}`);

  process.exit(drifted.length > 0 ? 1 : 0);
}

main().catch(err => {
  console.error('[probe] Fatal error:', err);
  process.exit(1);
});
