/**
 * CO-321: localhost trial flow
 *
 * E2E spec that exercises the full localhost trial flow across CO-313…CO-320:
 *   1. Anonymous boot — sidebar shows only universe + project list
 *   2. Subscribe — happy path (universe appears in subscribed bucket with × button)
 *   3. Subscribe — blocked on public-static (API returns 400; excluded from discoverable)
 *   4. Unsubscribe — happy path (× click removes row from sidebar)
 *   5. Theme persistence — chosen theme survives universe navigation (CO-313)
 *   6. Sister repo seeding — pages + tasks from CO_LOCAL_REPOS_DIR (CO-317/318/319)
 *   7. Subscribe button — discoverable list excludes static/private/own/subscribed
 *
 * TestServer (tests 2–7 except 6): the globally-bootstrapped server at
 * http://localhost:3000 with CO_ENV=test (started by global-setup.ts).
 *
 * TestServer (test 6): a separate ephemeral co-web process spawned with
 * CO_LOCAL_REPOS_DIR pointing to a tempdir containing mock sister-repo content.
 */

import { test, expect } from './fixtures';
import { openSidebarIfMobile } from './helpers';
import { spawn } from 'child_process';
import { mkdtempSync, writeFileSync, mkdirSync, rmSync } from 'fs';
import { join } from 'path';
import { tmpdir } from 'os';
import * as net from 'net';

// ─── Local TestServer helper for test 6 ─────────────────────────────────────
//
// Spawns a real co-web binary on an ephemeral port with custom env vars.
// Each server gets its own tempdir for state; they are independent.

interface TestServer {
  baseUrl: string;
  dispose: () => void;
}

function freePort(): Promise<number> {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.listen(0, '127.0.0.1', () => {
      const port = (srv.address() as net.AddressInfo).port;
      srv.close(() => resolve(port));
    });
  });
}

async function startTestServer(extraEnv: Record<string, string> = {}): Promise<TestServer> {
  const port = await freePort();
  const coWebDir = join(__dirname, '..');
  const projectRoot = join(coWebDir, '..');
  const binary = join(projectRoot, 'target', 'debug', 'co-web');
  const dataDir = mkdtempSync(join(tmpdir(), 'co-e2e-'));

  const child = spawn(
    binary,
    ['--port', String(port), '--static-dir', join(coWebDir, 'static'), '--data', dataDir],
    {
      env: {
        ...process.env,
        RUST_LOG: 'error',
        CO_ENV: 'test',
        CO_BYPASS_RATE_LIMIT: '1',
        ...extraEnv,
      },
      stdio: 'ignore',
    },
  );

  const baseUrl = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const r = await fetch(`${baseUrl}/api/health`);
      const b = (await r.json()) as { status?: string };
      if (b.status === 'ok') break;
    } catch {
      /* not ready yet */
    }
    await new Promise((r) => setTimeout(r, 250));
  }

  return {
    baseUrl,
    dispose: () => {
      try { child.kill('SIGTERM'); } catch { /* already dead */ }
      try { rmSync(dataDir, { recursive: true, force: true }); } catch { /* already cleaned */ }
    },
  };
}

// ─── Subscription target ─────────────────────────────────────────────────────
//
// `comunicacao` is seeded by seed_admin_content_universes as
// visibility='public-subscribable', owner_id='system'.
// It is NOT in yuri's ensure_admin_universe_memberships list, so
// yuri is neither owner nor member — subscribing makes it appear
// in the "subscribed" sidebar bucket (the only bucket with a × button).
const SUB_TARGET = 'comunicacao';

// ─── Tests ───────────────────────────────────────────────────────────────────

test.describe.configure({ mode: 'serial' });

test.describe('CO-321: localhost trial flow', () => {

  // ── 1. Anonymous boot ───────────────────────────────────────────────────────
  test('1. anonymous boot — no Plataformas, no raw chip key, no Descobríveis section', async ({ page }) => {
    // Fresh incognito-like visit: page has no session cookie by default.
    await page.goto('/', { waitUntil: 'networkidle' });
    const sidebar = page.locator('#project-list');

    // CO-311: Platforms/Plataformas section was removed — must not appear.
    await expect(sidebar.getByText(/plataforma/i)).toHaveCount(0);

    // CO-319: no raw i18n key "sidebar.co_dev_chip" rendered in the sidebar.
    await expect(page.getByText('sidebar.co_dev_chip')).toHaveCount(0);

    // CO-320: the old "Descobríveis" section label is gone.
    // (Replaced by a compact CTA button, shown only when logged-in + non-empty.)
    await expect(
      sidebar.locator('.sidebar-section-label').filter({ hasText: /descobr/i }),
    ).toHaveCount(0);

    // Anonymous users have no discoverable list → no "+ Buscar universos" button.
    await expect(sidebar.locator('.sidebar-discover-search-btn')).toHaveCount(0);
  });

  // ── 2. Subscribe — happy path ───────────────────────────────────────────────
  test('2. subscribe — universe appears in subscribed bucket with × button', async ({
    page,
    apiContext,
  }) => {
    // Idempotent cleanup: unsubscribe in case a previous test run left state.
    await apiContext.delete(`/api/v1/universes/${SUB_TARGET}/subscribe`);

    // Subscribe via API (apiContext is pre-authenticated as yuri@uat.local).
    const subRes = await apiContext.post(`/api/v1/universes/${SUB_TARGET}/subscribe`);
    expect(subRes.status()).toBe(204);

    // Authenticate the browser page so the sidebar renders the logged-in view.
    await page.request.post('/api/v1/auth/uat-login', {
      data: { email: 'yuri@uat.local', password: 'uat' },
    });
    await page.goto('/', { waitUntil: 'networkidle' });
    await openSidebarIfMobile(page);

    // Universe row must appear somewhere in the sidebar.
    const row = page.locator(`.sidebar-universe-item[data-universe="${SUB_TARGET}"]`);
    await expect(row).toBeVisible({ timeout: 8_000 });

    // CO-320: × unsubscribe button is rendered on the subscribed-bucket row.
    await expect(
      page.locator(`.sidebar-unsubscribe-btn[data-key="${SUB_TARGET}"]`),
    ).toBeVisible();

    // Cleanup.
    await apiContext.delete(`/api/v1/universes/${SUB_TARGET}/subscribe`);
  });

  // ── 3. Subscribe blocked on public-static ──────────────────────────────────
  test('3. subscribe blocked — public-static returns 400, excluded from discoverable', async ({
    apiContext,
  }) => {
    // `tempo` is a public-static (timeline trio) universe — users cannot subscribe.
    const subRes = await apiContext.post('/api/v1/universes/tempo/subscribe');
    expect(subRes.status()).toBe(400);
    const body = (await subRes.json()) as Record<string, unknown>;
    expect(JSON.stringify(body)).toMatch(/not public.?subscribable/i);

    // CO-319: list_discoverable_universes filters to visibility='public-subscribable'.
    // The timeline trio (tempo, humanity, universo) must NOT appear in the list.
    const meRes = await apiContext.get('/api/v1/me/universes');
    expect(meRes.status()).toBe(200);
    const me = (await meRes.json()) as { discoverable?: Array<{ key: string }> };
    const discKeys = (me.discoverable ?? []).map((u) => u.key);
    expect(discKeys).not.toContain('tempo');
    expect(discKeys).not.toContain('humanity');
    expect(discKeys).not.toContain('universo');
  });

  // ── 4. Unsubscribe — happy path ─────────────────────────────────────────────
  test('4. unsubscribe — clicking × removes universe from subscribed bucket', async ({
    page,
    apiContext,
  }) => {
    // Setup: subscribe so the × button appears.
    await apiContext.post(`/api/v1/universes/${SUB_TARGET}/subscribe`);

    await page.request.post('/api/v1/auth/uat-login', {
      data: { email: 'yuri@uat.local', password: 'uat' },
    });
    await page.goto('/', { waitUntil: 'networkidle' });
    await openSidebarIfMobile(page);

    const btn = page.locator(`.sidebar-unsubscribe-btn[data-key="${SUB_TARGET}"]`);
    await expect(btn).toBeVisible({ timeout: 8_000 });

    // CO-320: × triggers window.confirm; accept it, then DELETE subscribe is fired.
    page.on('dialog', (d) => d.accept());
    await btn.click();

    // The × button (and the subscribed row) must disappear after unsubscribe.
    await expect(
      page.locator(`.sidebar-unsubscribe-btn[data-key="${SUB_TARGET}"]`),
    ).toHaveCount(0, { timeout: 5_000 });
  });

  // ── 5. Theme persistence ────────────────────────────────────────────────────
  test('5. theme persistence — chosen theme survives universe navigation', async ({ page }) => {
    await page.request.post('/api/v1/auth/uat-login', {
      data: { email: 'yuri@uat.local', password: 'uat' },
    });
    await page.goto('/', { waitUntil: 'networkidle' });

    // Open the palette switcher and select 'scholarly-dark'.
    await page.locator('#palette-switcher-toggle').click();
    const btn = page.locator('[data-palette="scholarly-dark"]').first();
    test.skip((await btn.count()) === 0, 'scholarly-dark palette button not found in switcher');
    await btn.click();
    await expect(page.locator('html')).toHaveAttribute('data-palette', 'scholarly-dark', {
      timeout: 3_000,
    });

    // Navigate to a different universe — CO-313 removed per-universe theme overrides.
    // The palette must remain whatever the user last set, not reset on navigation.
    await page.goto('/template', { waitUntil: 'networkidle' });
    await expect(page.locator('html')).toHaveAttribute('data-palette', 'scholarly-dark');
  });

  // ── 6. Sister repo seeding — pages + tasks ──────────────────────────────────
  test('6. sister repo seeding — pages + tasks ingested from CO_LOCAL_REPOS_DIR', async () => {
    // Build a mock sister-repo tree using the hardcoded ArteLonga → artelonga mapping.
    // seed_orchestrator.run_sister_repo_seeds() iterates &[("artelonga","ArteLonga"),...]
    // and calls seed_universe_from_local_repo + seed_universe_work_tasks_from_local.
    const reposDir = mkdtempSync(join(tmpdir(), 'co-sister-'));
    const repoDir = join(reposDir, 'ArteLonga');
    mkdirSync(join(repoDir, 'docs'), { recursive: true });
    mkdirSync(join(repoDir, 'work', 'space'), { recursive: true });
    writeFileSync(join(repoDir, 'docs', 'page.md'), '---\ntitle: Test Page\n---\n\nHello\n');
    writeFileSync(
      join(repoDir, 'work', 'space', 'TEST-1.md'),
      '---\ntitle: Task One\ntype: task\nstatus: todo\n---\n',
    );

    const server = await startTestServer({ CO_LOCAL_REPOS_DIR: reposDir });
    let server1Alive = true;
    try {
      // Authenticate on the ephemeral server.
      const loginRes = await fetch(`${server.baseUrl}/api/v1/auth/uat-login`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email: 'yuri@uat.local', password: 'uat' }),
      });
      const cookie = (loginRes.headers.get('set-cookie') ?? '').split(';')[0];

      // CO-317: docs/page.md must appear in the artelonga universe entries.
      const r1 = await fetch(`${server.baseUrl}/api/v1/universes/artelonga/entries`, {
        headers: { Cookie: cookie },
      });
      expect(r1.status).toBe(200);
      const d1 = (await r1.json()) as {
        entries?: Array<{ path: string; entry_type?: string }>;
      };
      const entries1 = d1.entries ?? (d1 as unknown as Array<{ path: string; entry_type?: string }>);
      const paths1 = entries1.map((e) => e.path);
      expect(paths1).toContain('docs/page.md');

      // CO-318: TEST-1.md is seeded as a task entry at projects/TEST/TEST-1.md.
      // is_task_filename("TEST-1.md") → true; project_key = "TEST".
      const taskEntry = entries1.find((e) => e.path === 'projects/TEST/TEST-1.md');
      expect(taskEntry).toBeDefined();
      expect(taskEntry?.entry_type).toBe('task');

      // CO-319: add TEST-2.md, restart the server — new file must be picked up
      // on the very next boot (skip_if_count_above=0 disables the "already seeded" guard).
      writeFileSync(
        join(repoDir, 'work', 'space', 'TEST-2.md'),
        '---\ntitle: Task Two\ntype: task\nstatus: todo\n---\n',
      );

      server.dispose();
      server1Alive = false;

      const server2 = await startTestServer({ CO_LOCAL_REPOS_DIR: reposDir });
      try {
        const loginRes2 = await fetch(`${server2.baseUrl}/api/v1/auth/uat-login`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ email: 'yuri@uat.local', password: 'uat' }),
        });
        const cookie2 = (loginRes2.headers.get('set-cookie') ?? '').split(';')[0];
        const r2 = await fetch(`${server2.baseUrl}/api/v1/universes/artelonga/entries`, {
          headers: { Cookie: cookie2 },
        });
        const d2 = (await r2.json()) as {
          entries?: Array<{ path: string }>;
        };
        const paths2 = (d2.entries ?? (d2 as unknown as Array<{ path: string }>)).map(
          (e) => e.path,
        );
        expect(paths2).toContain('projects/TEST/TEST-2.md');
      } finally {
        server2.dispose();
      }
    } finally {
      if (server1Alive) server.dispose();
      rmSync(reposDir, { recursive: true, force: true });
    }
  });

  // ── 7. Discoverable list — no static/private/own/subscribed universes ───────
  test('7. discoverable list — excludes static, private, own, and already-subscribed', async ({
    page,
    apiContext,
  }) => {
    // Ensure a clean subscription state.
    await apiContext.delete(`/api/v1/universes/${SUB_TARGET}/subscribe`);

    // ── API-level assertions ──────────────────────────────────────────────────
    const meRes = await apiContext.get('/api/v1/me/universes');
    expect(meRes.status()).toBe(200);
    const me = (await meRes.json()) as {
      owned?: Array<{ key: string }>;
      subscribed?: Array<{ key: string }>;
      discoverable?: Array<{ key: string }>;
    };
    const discKeys = (me.discoverable ?? []).map((u) => u.key);

    // Public-static (timeline trio) must never appear — users cannot subscribe to them.
    for (const k of ['tempo', 'humanity', 'universo']) {
      expect(discKeys, `'${k}' (public-static) must not be in discoverable`).not.toContain(k);
    }
    // Universes the user owns must not be in discoverable.
    for (const { key } of me.owned ?? []) {
      expect(discKeys, `owned '${key}' must not be in discoverable`).not.toContain(key);
    }
    // Already-subscribed universes must not be in discoverable.
    for (const { key } of me.subscribed ?? []) {
      expect(discKeys, `subscribed '${key}' must not be in discoverable`).not.toContain(key);
    }

    // ── UI-level assertion: search prompt lists only valid candidates ─────────
    if ((me.discoverable ?? []).length > 0) {
      await page.request.post('/api/v1/auth/uat-login', {
        data: { email: 'yuri@uat.local', password: 'uat' },
      });
      await page.goto('/', { waitUntil: 'networkidle' });
      await openSidebarIfMobile(page);

      const searchBtn = page.locator('.sidebar-discover-search-btn');
      if ((await searchBtn.count()) > 0) {
        let promptMsg = '';
        page.once('dialog', async (d) => {
          promptMsg = d.message();
          await d.dismiss();
        });
        await searchBtn.click();
        // Give the dialog a moment to fire and be captured.
        await page.waitForTimeout(300);
        if (promptMsg) {
          for (const k of ['tempo', 'humanity', 'universo']) {
            expect(promptMsg, `'${k}' (public-static) must not appear in prompt`).not.toContain(k);
          }
          for (const { key } of me.owned ?? []) {
            expect(promptMsg, `owned '${key}' must not appear in prompt`).not.toContain(key);
          }
        }
      }
    }
  });
});
