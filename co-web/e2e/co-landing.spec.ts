import { test, expect } from "./fixtures";

// CO-27: Landing page at /co — template board with hero, login, criar universo

test.describe("CO-27: /co landing page", () => {
  test("GET /co returns 200 with HTML", async ({ apiContext }) => {
    const res = await apiContext.get("/co");
    expect(res.status()).toBe(200);
    const ct = res.headers()["content-type"] || "";
    expect(ct).toContain("text/html");
  });

  test("GET /co/{slug} returns 200 with HTML", async ({ apiContext }) => {
    const res = await apiContext.get("/co/any-slug");
    expect(res.status()).toBe(200);
    const ct = res.headers()["content-type"] || "";
    expect(ct).toContain("text/html");
  });

  test("/co shows template banner with hero and CTA", async ({ page }) => {
    await page.goto("/co");
    // Template banner should be visible
    const banner = page.locator("#template-banner");
    await expect(banner).toBeVisible();
    // Logo mark visible
    await expect(banner.locator(".template-logo-mark")).toBeVisible();
    // Title visible
    await expect(banner.locator("#template-hero-title")).toBeVisible();
    // Primary CTA
    await expect(banner.locator("#btn-criar-universo")).toBeVisible();
    // Secondary Entrar
    await expect(banner.locator("#btn-banner-entrar")).toBeVisible();
    // GitHub link
    await expect(banner.locator("#btn-banner-github")).toBeVisible();
    // Language toggle
    await expect(banner.locator("#btn-banner-lang")).toBeVisible();
  });

  test("/co shows board below hero (kanban columns)", async ({ page }) => {
    await page.goto("/co");
    // Template banner visible
    const banner = page.locator("#template-banner");
    await expect(banner).toBeVisible();
    // Board content loads (kanban columns appear)
    await page.waitForSelector(".kanban-board", { timeout: 10_000 });
    const columns = page.locator(".kanban-col");
    await expect(columns).not.toHaveCount(0);
  });

  test("/co board interactions are disabled (no task modal on click)", async ({
    page,
  }) => {
    await page.goto("/co");
    await page.waitForSelector(".task-card", { timeout: 10_000 });
    // Clicking a task card should NOT open the task modal (template is read-only)
    const card = page.locator(".task-card").first();
    await card.click();
    const modal = page.locator("#modal-overlay");
    // Modal must not be visible
    await expect(modal).toHaveClass(/hidden/);
  });

  test("'Entrar' in banner opens login modal", async ({ page }) => {
    await page.goto("/co");
    await expect(page.locator("#template-banner")).toBeVisible();
    await page.locator("#btn-banner-entrar").click();
    await expect(page.locator("#login-modal-overlay")).not.toHaveClass(
      /hidden/,
    );
  });

  test("'Criar universo' opens criar modal with slug preview", async ({
    page,
  }) => {
    await page.goto("/co");
    await expect(page.locator("#template-banner")).toBeVisible();
    await page.locator("#btn-criar-universo").click();
    const modal = page.locator("#criar-modal-overlay");
    await expect(modal).not.toHaveClass(/hidden/);
    // Slug preview element present
    await expect(page.locator("#criar-slug-preview")).toBeAttached();
    // Fill name → slug auto-generates and preview updates
    await page.locator("#criar-name").fill("Meu Projeto");
    const preview = page.locator("#criar-slug-val");
    await expect(preview).not.toHaveText("");
  });

  test("footer is visible with version info", async ({ page }) => {
    await page.goto("/co");
    const footer = page.locator("#app-footer");
    await expect(footer).toBeVisible();
    await expect(footer.locator("#footer-version")).toBeVisible();
    // GitHub link present
    await expect(footer.locator(".footer-github-link")).toBeVisible();
  });

  test("SEO meta tags present", async ({ page }) => {
    await page.goto("/co");
    const ogTitle = await page
      .locator('meta[property="og:title"]')
      .getAttribute("content");
    expect(ogTitle).toBeTruthy();
    const description = await page
      .locator('meta[name="description"]')
      .getAttribute("content");
    expect(description).toBeTruthy();
  });

  test("language toggle in banner switches language", async ({ page }) => {
    await page.goto("/co");
    const banner = page.locator("#template-banner");
    await expect(banner).toBeVisible();
    // Default lang is pt — toggle should show "English"
    const langBtn = banner.locator("#btn-banner-lang");
    await expect(langBtn).toBeVisible();
    await langBtn.click();
    // After switching to English, button should show "Português"
    await expect(langBtn).toHaveText("Português");
  });
});
