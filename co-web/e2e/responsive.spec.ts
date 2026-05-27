/**
 * Responsive — thinned (CO-302): one test per breakpoint.
 * Tablet JS-error check and sidebar open/close moved to component tests.
 */

import { test, expect } from "./fixtures";

test.describe("Responsive: desktop 1280×720", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("hamburger NOT visible at 1280px", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    const hamburger = page.locator("#hamburger-btn");
    if (await hamburger.count() > 0) {
      await expect(hamburger).not.toBeVisible();
    }
  });
});

test.describe("Responsive: tablet 768×1024", () => {
  test.use({ viewport: { width: 768, height: 1024 } });

  test("page loads without JS errors at 768px", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto("/", { waitUntil: "networkidle" });
    expect(errors).toHaveLength(0);
  });
});

test.describe("Responsive: mobile 375×812", () => {
  test.use({ viewport: { width: 375, height: 812 } });

  test("hamburger visible at 375px", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await expect(page.locator("#hamburger-btn")).toBeVisible();
  });

  test("template hero renders on mobile", async ({ page }) => {
    // Template banner lives at "/" — "/co" boots the co dev universe (no banner)
    await page.goto("/", { waitUntil: "networkidle" });
    await expect(page.locator("#template-banner")).toBeVisible();
  });

  test("sidebar opens via hamburger", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    const hamburger = page.locator("#hamburger-btn");
    await expect(hamburger).toBeVisible();
    await hamburger.click();
    const sidebar = page.locator("#sidebar");
    await expect(sidebar).toBeVisible({ timeout: 2_000 });
  });
});
