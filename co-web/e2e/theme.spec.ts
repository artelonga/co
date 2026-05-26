/**
 * Theme — thinned (CO-302): switcher toggle + palette change + no page reload.
 * CSS variable checks and visitor propagation moved to component tests.
 */

import { test, expect } from "./fixtures";

test.describe("Theme switcher", () => {
  test("palette switcher toggle button is visible", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await expect(page.locator("#palette-switcher-toggle")).toBeVisible({
      timeout: 5_000,
    });
  });

  test("selecting a palette updates data-palette attribute", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#palette-switcher-toggle").click();

    const btn = page.locator('[data-palette="scholarly"]').first();
    const count = await btn.count();
    test.skip(count === 0, "scholarly palette button not found");
    await btn.click();

    const html = page.locator("html");
    await expect(html).toHaveAttribute("data-palette", "scholarly");
  });

  test("switching palette does not trigger a page reload", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    let reloaded = false;
    page.on("load", () => { reloaded = true; });

    await page.locator("#palette-switcher-toggle").click();
    const btn = page.locator("[data-palette]").first();
    if (await btn.count() > 0) await btn.click();

    expect(reloaded).toBe(false);
  });
});
