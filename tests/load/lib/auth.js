import http from 'k6/http';
import { check } from 'k6';
import exec from 'k6/execution';

export function guardProd(baseUrl) {
  const isProd =
    baseUrl.includes('co.artelonga.com.br') ||
    (baseUrl.includes('co-artelonga.fly.dev') && !baseUrl.includes('-uat'));
  if (isProd) {
    exec.test.abort(`ABORT: refusing to run against production (${baseUrl}). Use UAT (co-artelonga-uat.fly.dev).`);
  }
}

/**
 * Login once in setup(). Returns the raw session JWT so all VUs can use it as a
 * Cookie header without triggering a login stampede at iteration 0.
 */
export function uatLoginSetup(baseUrl) {
  const res = http.post(
    `${baseUrl}/api/v1/auth/uat-login`,
    JSON.stringify({ email: 'yuri@uat.local', password: 'uat' }),
    { headers: { 'Content-Type': 'application/json' } },
  );
  if (res.status !== 200) {
    exec.test.abort(`UAT login failed in setup: ${res.status} — ${res.body}`);
  }
  // Extract the session cookie set by the server.
  const sessionCookie = res.cookies.session && res.cookies.session[0]
    ? res.cookies.session[0].value
    : null;
  if (!sessionCookie) {
    exec.test.abort('UAT login did not return a session cookie.');
  }
  return sessionCookie;
}

/**
 * Provision an API token for vault scenarios. Must be called from setup() after
 * uatLoginSetup() so the session cookie is in the VU jar.
 */
export function provisionApiToken(baseUrl) {
  const res = http.post(
    `${baseUrl}/api/v1/auth/token`,
    JSON.stringify({}),
    { headers: { 'Content-Type': 'application/json' } },
  );
  check(res, { 'token 2xx': (r) => r.status >= 200 && r.status < 300 });
  if (res.status < 200 || res.status >= 300) {
    exec.test.abort(`Token provisioning failed: ${res.status} — ${res.body}`);
  }
  return res.json().token;
}
