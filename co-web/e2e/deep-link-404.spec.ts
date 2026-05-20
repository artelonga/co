/**
 * CO-232 — Cross-universe deep-link returns universe home instead of 404
 *
 * Asserts that:
 * - `/<universe>/<unknown-slug>` returns HTTP 404 and renders the 404 SPA view.
 * - `/<universe>/` (no slug) continues to render the universe home (200).
 * - Covers at least two universes: `template` and `artelonga`.
 * - The 404 view contains links back to the universe home and the global home.
 */

import { test, expect, request as playwrightRequest } from "@playwright/test";

const UNIVERSES = ["template", "artelonga"];
const UNKNOWN_SLUG = "this-entry-does-not-exist-co232";

test.describe("CO-232: deep-link 404 for unknown entries", () => {
  for (const universe of UNIVERSES) {
    test(`/${universe}/${UNKNOWN_SLUG} returns HTTP 404`, async () => {
      const base = process.env.BASE_URL ?? "http://localhost:3000";
      const ctx = await playwrightRequest.newContext({ baseURL: base });

      const resp = await ctx.get(`/${universe}/${UNKNOWN_SLUG}`);
      expect(
        resp.status(),
        `expected 404 for /${universe}/${UNKNOWN_SLUG}`,
      ).toBe(404);

      await ctx.dispose();
    });

    test(`/${universe}/${UNKNOWN_SLUG} renders the 404 SPA view with recovery links`, async ({
      page,
    }) => {
      await page.goto(`/${universe}/${UNKNOWN_SLUG}`, {
        waitUntil: "domcontentloaded",
      });

      // 404 view must appear
      const notFoundView = page.locator("#co-not-found-view");
      await expect(notFoundView).toBeVisible({ timeout: 10_000 });

      // Must contain a link back to the universe home
      const universeLink = notFoundView.locator(`a[href="/${universe}"]`);
      await expect(universeLink).toBeVisible();

      // Must contain a link back to the global home
      const homeLink = notFoundView.locator('a[href="/"]');
      await expect(homeLink).toBeVisible();
    });
  }

  test("/template/ (no slug) continues to render universe home — not 404", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });

    const resp = await ctx.get("/template");
    expect(resp.status(), "universe home must be 200").toBe(200);

    await ctx.dispose();
  });
});
