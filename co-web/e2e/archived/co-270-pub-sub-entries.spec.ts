/**
 * CO-270 — public-subscribable universe entries visible to anonymous visitors
 *
 * Verifies the fix to universe_visibility_gate: anonymous visitors can list
 * entries in a public-subscribable universe (/co) and see rendered cards in
 * the kanban view.
 */

import { test, expect } from "./fixtures";

const PUB_SUB_SLUG = "co";

test.describe("CO-270: anonymous entry visibility on public-subscribable universe", () => {
  test("GET /api/v1/universes/co/entries returns total > 0 and non-empty items", async ({
    apiContext,
  }) => {
    const res = await apiContext.get(
      `/api/v1/universes/${PUB_SUB_SLUG}/entries?limit=3`,
    );
    expect(res.status()).toBe(200);

    const body = await res.json();
    expect(body.total).toBeGreaterThan(0);
    expect(Array.isArray(body.entries)).toBe(true);
    expect(body.entries.length).toBeGreaterThan(0);
    expect(body.entries.length).toBeLessThanOrEqual(3);
  });

  test("anonymous /co board renders at least one entry card", async ({
    page,
  }) => {
    await page.goto(`/${PUB_SUB_SLUG}`, { waitUntil: "networkidle" });

    // At least one task card must be visible after the board loads.
    const firstCard = page.locator(".task-card").first();
    await expect(firstCard).toBeVisible({ timeout: 10_000 });
  });
});
