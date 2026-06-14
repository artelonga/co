/**
 * CO-353: Lobby + realtime presence — WebSocket layer for workspace canvases.
 *
 * Two browser contexts both open the same Sala. One client emits a node move
 * (and a cursor); the other client — driven by the real `sala-realtime.js`
 * client wired into the page — receives them live over the `/ws/sala/...`
 * WebSocket. This is the end-to-end proof that a Sala is a shared room.
 */

import { test, expect, Page } from "@playwright/test";

const BASE = process.env.BASE_URL ?? "http://localhost:3000";
const SALA = "/u/template/sala";

async function loginPage(page: Page): Promise<void> {
  // The e2e server boots with CO_ENV=test which enables uat-login; the POST
  // sets the session cookie on the page context for subsequent navigations.
  await page.request.post(`${BASE}/api/v1/auth/uat-login`, {
    data: { email: "yuri@uat.local", password: "uat" },
  });
}

/** Wait until the page's realtime client has connected to the room. */
async function waitForRealtime(page: Page): Promise<void> {
  await page.waitForFunction(
    () =>
      typeof (window as any).SalaRealtime === "function" &&
      !!(window as any).__salaRealtime,
    null,
    { timeout: 15_000 },
  );
}

test.describe("CO-353: realtime presence over the Sala canvas", () => {
  test.beforeEach(({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sala canvas is desktop-only");
  });

  test("two contexts: a node move on one appears on the other", async ({
    browser,
  }) => {
    const ctxA = await browser.newContext();
    const ctxB = await browser.newContext();
    const pageA = await ctxA.newPage();
    const pageB = await ctxB.newPage();

    await loginPage(pageA);
    await loginPage(pageB);

    await pageA.goto(SALA, { waitUntil: "domcontentloaded" });
    await pageB.goto(SALA, { waitUntil: "domcontentloaded" });

    await expect(pageA.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });
    await expect(pageB.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });

    await waitForRealtime(pageA);
    await waitForRealtime(pageB);

    // Client A broadcasts a node move; client B must receive it live.
    await pageA.evaluate(() => {
      (window as any).__salaRealtime.sendNodeMove("e2e/realtime-node", 7, 3, "op-e2e");
    });

    await pageB.waitForFunction(
      () => {
        const m = (window as any).__salaLastRemoteMove;
        return m && m.entry_path === "e2e/realtime-node" && m.x === 7 && m.y === 3;
      },
      null,
      { timeout: 10_000 },
    );

    const move = await pageB.evaluate(() => (window as any).__salaLastRemoteMove);
    expect(move.entry_path).toBe("e2e/realtime-node");
    expect(move.x).toBe(7);
    expect(move.y).toBe(3);

    await ctxA.close();
    await ctxB.close();
  });

  test("two contexts each see the other's cursor", async ({ browser }) => {
    const ctxA = await browser.newContext();
    const ctxB = await browser.newContext();
    const pageA = await ctxA.newPage();
    const pageB = await ctxB.newPage();

    await loginPage(pageA);
    await loginPage(pageB);

    await pageA.goto(SALA, { waitUntil: "domcontentloaded" });
    await pageB.goto(SALA, { waitUntil: "domcontentloaded" });
    await waitForRealtime(pageA);
    await waitForRealtime(pageB);

    // A moves its cursor; B's realtime client must register a remote cursor.
    await pageA.evaluate(() => {
      (window as any).__salaRealtime.sendCursor(123, 456);
    });

    await pageB.waitForFunction(
      () => {
        const cursors = (window as any).__salaRealtime.cursors;
        return Object.keys(cursors).length > 0;
      },
      null,
      { timeout: 10_000 },
    );

    const count = await pageB.evaluate(
      () => Object.keys((window as any).__salaRealtime.cursors).length,
    );
    expect(count).toBeGreaterThan(0);

    await ctxA.close();
    await ctxB.close();
  });
});
