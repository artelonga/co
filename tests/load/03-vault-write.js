/**
 * 03-vault-write.js — API token holder (Obsidian-compat vault read/write/delete)
 *
 * Simulates an Obsidian plugin or CLI tool syncing notes via the Vault REST API.
 * Uses a long-lived API token provisioned once in setup. Each VU loops through
 * list → write → delete using unique per-VU-per-iter filenames.
 *
 * The `vaulttest` universe is created in setup (idempotent) and used for all
 * writes — template stays clean, quilomboaraucaria stays clean.
 *
 * Usage:
 *   k6 run --vus 50 --duration 1m tests/load/03-vault-write.js
 *   BASE_URL=https://co-artelonga-uat.fly.dev k6 run --vus 100 --duration 1m tests/load/03-vault-write.js
 */

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Trend } from 'k6/metrics';
import { guardProd, uatLoginSetup, provisionApiToken } from './lib/auth.js';
import { baseThresholds } from './lib/thresholds.js';

const BASE_URL = __ENV.BASE_URL || 'https://co-artelonga-uat.fly.dev';
const VAULT_UNIVERSE = 'vaulttest';
const LOAD_TEST_DIR = 'load-test';

const vaultWriteLatency = new Trend('vault_write_ms');

export const options = {
  vus: 50,
  duration: '1m',
  thresholds: {
    ...baseThresholds,
    vault_write_ms: ['p(95)<2000'],
  },
};

export function setup() {
  guardProd(BASE_URL);

  const sessionCookie = uatLoginSetup(BASE_URL);
  const sessionHeaders = { Cookie: `session=${sessionCookie}`, 'Content-Type': 'application/json' };

  // Create vault test universe (409 = already exists = OK).
  const createRes = http.post(
    `${BASE_URL}/api/v1/universes`,
    JSON.stringify({ name: 'Vault Load Test', key: VAULT_UNIVERSE, description: 'k6 scratch space' }),
    { headers: sessionHeaders },
  );
  if (createRes.status !== 201 && createRes.status !== 409) {
    const _ = check(createRes, { 'universe created or exists': () => false });
  }

  // Provision API token (session cookie in VU jar from uatLoginSetup above).
  const token = provisionApiToken(BASE_URL);
  return { token };
}

export default function (data) {
  const authHeaders = {
    Authorization: `Bearer ${data.token}`,
    'Content-Type': 'text/plain',
  };
  const filePath = `${LOAD_TEST_DIR}/vu${__VU}-iter${__ITER}.md`;

  // --- List vault ---
  check(
    http.get(`${BASE_URL}/api/v1/universes/${VAULT_UNIVERSE}/vault/`, {
      headers: { Authorization: `Bearer ${data.token}` },
    }),
    { 'vault list 200': (r) => r.status === 200 },
  );

  // --- Write file ---
  const t0 = Date.now();
  const content = `# Load test\n\nVU: ${__VU}  Iter: ${__ITER}  Time: ${new Date().toISOString()}\n`;

  check(
    http.put(
      `${BASE_URL}/api/v1/universes/${VAULT_UNIVERSE}/vault/${filePath}`,
      content,
      { headers: authHeaders },
    ),
    { 'vault write 2xx': (r) => r.status >= 200 && r.status < 300 },
  );

  vaultWriteLatency.add(Date.now() - t0);

  // --- Delete file (cleanup — no lingering test data) ---
  check(
    http.del(
      `${BASE_URL}/api/v1/universes/${VAULT_UNIVERSE}/vault/${filePath}`,
      null,
      { headers: { Authorization: `Bearer ${data.token}` } },
    ),
    { 'vault delete 2xx': (r) => r.status >= 200 && r.status < 300 },
  );

  sleep(2 + Math.random() * 3);
}
