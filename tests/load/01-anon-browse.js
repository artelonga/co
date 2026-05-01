/**
 * 01-anon-browse.js — anonymous landing flow
 *
 * Simulates a first-time visitor browsing the board and timeline.
 * No auth required. Reads only — no writes to UAT data.
 *
 * Usage:
 *   k6 run --vus 50 --duration 1m tests/load/01-anon-browse.js
 *   BASE_URL=https://co-artelonga-uat.fly.dev k6 run --vus 100 --duration 1m tests/load/01-anon-browse.js
 */

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Trend } from 'k6/metrics';
import { guardProd } from './lib/auth.js';
import { baseThresholds } from './lib/thresholds.js';

const BASE_URL = __ENV.BASE_URL || 'https://co-artelonga-uat.fly.dev';

const boardLoadTime = new Trend('board_load_ms');
const timelineLoadTime = new Trend('timeline_load_ms');

export const options = {
  vus: 50,
  duration: '1m',
  thresholds: baseThresholds,
};

export function setup() {
  guardProd(BASE_URL);
}

export default function () {
  // --- Board landing ---

  let t0 = Date.now();

  check(http.get(`${BASE_URL}/`), { 'GET / 200': (r) => r.status === 200 });
  check(http.get(`${BASE_URL}/app.js`), { 'GET /app.js 200': (r) => r.status === 200 });
  check(http.get(`${BASE_URL}/style.css`), { 'GET /style.css 200': (r) => r.status === 200 });

  const universe = http.get(`${BASE_URL}/api/v1/universes/template`);
  check(universe, { 'template universe 200': (r) => r.status === 200 });

  check(http.get(`${BASE_URL}/api/v1/universes/template/config`), {
    'template config 200': (r) => r.status === 200,
  });
  check(http.get(`${BASE_URL}/api/v1/universes/template/theme.css`), {
    'theme.css 200': (r) => r.status === 200,
  });
  check(http.get(`${BASE_URL}/api/v1/universes/template/projects`), {
    'template projects 200': (r) => r.status === 200,
  });
  check(http.get(`${BASE_URL}/api/projects/CO/tasks?u=template`), {
    'CO tasks 200': (r) => r.status === 200,
  });

  boardLoadTime.add(Date.now() - t0);

  sleep(3 + Math.random() * 4);

  // --- Timeline browsing ---

  t0 = Date.now();

  check(http.get(`${BASE_URL}/shared/timeline.html?u=tempo`), {
    'timeline.html 200': (r) => r.status === 200,
  });
  check(http.get(`${BASE_URL}/api/v1/universes/tempo/entries?type=event`), {
    'tempo events 200': (r) => r.status === 200,
  });
  check(http.get(`${BASE_URL}/api/v1/universes/universo/entries?type=event`), {
    'universo events 200': (r) => r.status === 200,
  });
  check(http.get(`${BASE_URL}/api/v1/universes/humanity/entries?type=event`), {
    'humanity events 200': (r) => r.status === 200,
  });

  timelineLoadTime.add(Date.now() - t0);

  sleep(5 + Math.random() * 5);
}
