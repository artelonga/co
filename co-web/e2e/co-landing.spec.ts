/**
 * /co landing page — thinned (CO-302): HTTP + banner + board + CTAs.
 * Footer, SEO, language toggle moved to component tests.
 */

import { test, expect } from "./fixtures";

test.describe("CO-27: /co landing", () => {
  test("GET /co returns 200 HTML", async ({ apiContext }) => {
    const res = await apiContext.get("/co");
    expect(res.status()).toBe(200);
    expect(res.headers()["content-type"]).toContain("text/html");
  });

  test("template banner with hero and CTA is visible", async ({ page }) => {
    await page.goto("/co");
    await expect(page.locator("#template-banner")).toBeVisible();
    await expect(page.locator("#btn-criar-universo")).toBeVisible();
    await expect(page.locator("#btn-banner-entrar")).toBeVisible();
  });

  test("board renders below hero with kanban columns", async ({ page }) => {
    await page.goto("/co");
    await page.waitForSelector(".kanban-board", { timeout: 10_000 });
    await expect(page.locator(".kanban-col")).not.toHaveCount(0);
  });

  test("'Entrar' opens login modal", async ({ page }) => {
    await page.goto("/co");
    await expect(page.locator("#template-banner")).toBeVisible();
    await page.locator("#btn-banner-entrar").click();
    await expect(page.locator("#login-modal-overlay")).not.toHaveClass(/hidden/);
  });

  test("'Criar universo' opens criar modal", async ({ page }) => {
    await page.goto("/co");
    await page.locator("#btn-criar-universo").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);
  });
});
