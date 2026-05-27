/**
 * Landing page — thinned (CO-302): HTTP + banner + board + CTAs.
 * Footer, SEO, language toggle moved to component tests.
 *
 * Note (CO-304): the template banner lives at "/" (template universe), not
 * "/co" — the co universe is a real registered universe seeded at startup
 * (seed_admin_content_universes), so /co boots the co board without a banner.
 */

import { test, expect } from "./fixtures";

test.describe("CO-27: landing page", () => {
  test("GET /co returns 200 HTML", async ({ apiContext }) => {
    const res = await apiContext.get("/co");
    expect(res.status()).toBe(200);
    expect(res.headers()["content-type"]).toContain("text/html");
  });

  test("template banner with hero and CTA is visible", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await expect(page.locator("#template-banner")).toBeVisible();
    await expect(page.locator("#btn-criar-universo")).toBeVisible();
    await expect(page.locator("#btn-banner-entrar")).toBeVisible();
  });

  test("board renders below hero with kanban columns", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    // Default view is conteudo — switch to kanban to verify the board renders.
    // kanban.js renders <div class="kanban"> with <div class="kanban-column">
    await page.locator('#view-tabs .view-tab[data-view="kanban"]').click();
    await page.waitForSelector(".kanban", { timeout: 10_000 });
    await expect(page.locator(".kanban-column")).not.toHaveCount(0);
  });

  test("'Entrar' opens login modal", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await expect(page.locator("#template-banner")).toBeVisible();
    await page.locator("#btn-banner-entrar").click();
    await expect(page.locator("#login-modal-overlay")).not.toHaveClass(/hidden/);
  });

  test("'Criar universo' opens criar modal", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#btn-criar-universo").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);
  });
});
