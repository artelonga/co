/**
 * 02-logged-in-board.js — power user (yuri-style)
 *
 * Simulates an authenticated user browsing universes and doing board writes.
 * Login happens once in setup() to avoid a concurrent-login stampede.
 * The session cookie is extracted from the login response and injected as a
 * request header for all VUs.
 *
 * Tasks are created and immediately deleted to avoid hitting the usage gate.
 * The LDTEST project must exist on UAT (created once in setup).
 *
 * Usage:
 *   k6 run --vus 50 --duration 1m tests/load/02-logged-in-board.js
 *   BASE_URL=https://co-artelonga-uat.fly.dev k6 run --vus 100 --duration 1m tests/load/02-logged-in-board.js
 */

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Trend } from 'k6/metrics';
import { guardProd, uatLoginSetup } from './lib/auth.js';
import { baseThresholds } from './lib/thresholds.js';

const BASE_URL = __ENV.BASE_URL || 'https://co-artelonga-uat.fly.dev';

const writeLatency = new Trend('board_write_ms');

export const options = {
  vus: 50,
  duration: '1m',
  thresholds: {
    ...baseThresholds,
    board_write_ms: ['p(95)<2000'],
  },
};

const WRITE_PROJECT = 'LDTEST';

export function setup() {
  guardProd(BASE_URL);
  const sessionCookie = uatLoginSetup(BASE_URL);

  const authHeaders = { Cookie: `session=${sessionCookie}`, 'Content-Type': 'application/json' };

  // Ensure the write project exists (idempotent — 409 if already present is fine).
  http.post(
    `${BASE_URL}/api/projects`,
    JSON.stringify({ name: 'Load Test', key: WRITE_PROJECT, description: 'k6 scratch space' }),
    { headers: authHeaders },
  );

  return { sessionCookie };
}

export default function (data) {
  const authHeaders = {
    Cookie: `session=${data.sessionCookie}`,
    'Content-Type': 'application/json',
  };

  // --- Auth / me ---
  check(http.get(`${BASE_URL}/api/v1/auth/me`, { headers: authHeaders }), {
    'me 200': (r) => r.status === 200,
  });

  // --- List all universes ---
  const universesRes = http.get(`${BASE_URL}/api/v1/universes`, { headers: authHeaders });
  check(universesRes, { 'universes 200': (r) => r.status === 200 });

  const universes = universesRes.status === 200 ? universesRes.json() : [];
  const browseKeys = universes.slice(0, 3).map((u) => u.key);

  // --- Browse up to 3 universes ---
  for (const key of browseKeys) {
    check(http.get(`${BASE_URL}/api/v1/universes/${key}`, { headers: authHeaders }), {
      'universe info 200': (r) => r.status === 200,
    });
    check(http.get(`${BASE_URL}/api/v1/universes/${key}/config`, { headers: authHeaders }), {
      'universe config 2xx': (r) => r.status >= 200 && r.status < 300,
    });

    const projectsRes = http.get(`${BASE_URL}/api/v1/universes/${key}/projects`, { headers: authHeaders });
    check(projectsRes, { 'projects 200': (r) => r.status === 200 });

    const projects = projectsRes.status === 200 ? projectsRes.json() : [];
    if (projects.length > 0) {
      check(http.get(`${BASE_URL}/api/projects/${projects[0].key}/tasks`, { headers: authHeaders }), {
        'tasks 200': (r) => r.status === 200,
      });
    }
  }

  sleep(2 + Math.random() * 3);

  // --- Board write: create → update → delete in LDTEST ---
  const t0 = Date.now();

  const createRes = http.post(
    `${BASE_URL}/api/projects/LDTEST/tasks`,
    JSON.stringify({
      title: `load-test-${__VU}-${__ITER}`,
      status: 'todo',
      priority: 'low',
    }),
    { headers: authHeaders },
  );
  check(createRes, { 'create task 201': (r) => r.status === 201 });

  if (createRes.status === 201) {
    const taskId = createRes.json().id;

    check(
      http.put(
        `${BASE_URL}/api/projects/LDTEST/tasks/${taskId}`,
        JSON.stringify({ status: 'done' }),
        { headers: authHeaders },
      ),
      { 'update task 200': (r) => r.status === 200 },
    );

    // Clean up — keeps UAT data tidy and avoids hitting the usage gate.
    http.del(`${BASE_URL}/api/projects/LDTEST/tasks/${taskId}`, null, {
      headers: authHeaders,
    });
  }

  writeLatency.add(Date.now() - t0);

  sleep(4 + Math.random() * 4);
}
