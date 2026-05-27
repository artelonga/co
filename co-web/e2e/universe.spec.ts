/**
 * CO-33 — Universe creation: Criar universo submit flow
 *
 * Covers:
 *   - Clicking "Criar universo" → filling name/slug → submitting form
 *   - Redirect to /<slug> after creation (SPA pushState, not /co/:slug)
 *   - Resulting board is editable (not read-only like the template)
 *
 * Note (CO-304): template banner lives at "/" (template universe). "/co" boots
 * the co dev board which has no template banner — navigate to "/" instead.
 */

import { test, expect } from "./fixtures";

test.describe("Universe creation: submit form → redirect → editable board", () => {
  test("clicking 'Criar universo' opens the criar modal", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await expect(page.locator("#template-banner")).toBeVisible();

    await page.locator("#btn-criar-universo").click();

    const modal = page.locator("#criar-modal-overlay");
    await expect(modal).not.toHaveClass(/hidden/);
    await expect(page.locator("#criar-name")).toBeVisible();
    await expect(page.locator("#criar-slug")).toBeVisible();
  });

  test("typing a name auto-fills the slug input", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#btn-criar-universo").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);

    await page.locator("#criar-name").fill("Meu Projeto Teste");

    // Slug should be auto-generated and preview updated
    const slugVal = page.locator("#criar-slug-val");
    await expect(slugVal).not.toHaveText("");

    const slugInput = page.locator("#criar-slug");
    const slugValue = await slugInput.inputValue();
    expect(slugValue).toMatch(/^[a-z0-9-]+$/);
  });

  test("submitting the criar form redirects to /<slug>", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#btn-criar-universo").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);

    const slug = `e2e-univ-${Date.now()}`;
    await page.locator("#criar-name").fill("E2E Universo");
    await page.locator("#criar-slug").fill(slug);

    await page.locator("#criar-form").dispatchEvent("submit");

    // SPA uses pushState(slug) → URL becomes /<slug>, not /co/<slug>
    await page.waitForURL(`**/${slug}`, { timeout: 15_000 });
    expect(page.url()).toContain(`/${slug}`);
  });

  test("after universe creation the board is editable: '+ Nova Tarefa' button visible", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#btn-criar-universo").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);

    const slug = `e2e-edit-${Date.now()}`;
    await page.locator("#criar-name").fill("Universo Editável");
    await page.locator("#criar-slug").fill(slug);
    await page.locator("#criar-form").dispatchEvent("submit");

    await page.waitForURL(`**/${slug}`, { timeout: 15_000 });

    // In an owned universe the New Task button should appear
    const newBtn = page.locator("#btn-new-task");
    await expect(newBtn).toBeVisible({ timeout: 10_000 });
  });

  test("after universe creation the template banner is NOT shown (board is owned)", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#btn-criar-universo").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);

    const slug = `e2e-nobanner-${Date.now()}`;
    await page.locator("#criar-name").fill("No Banner Universe");
    await page.locator("#criar-slug").fill(slug);
    await page.locator("#criar-form").dispatchEvent("submit");

    await page.waitForURL(`**/${slug}`, { timeout: 15_000 });

    // Template banner should be hidden on the owned universe
    const banner = page.locator("#template-banner");
    await expect(banner).toHaveClass(/hidden/);
  });

  test("cancelling the criar modal closes it without navigating", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    const initialUrl = page.url();

    await page.locator("#btn-criar-universo").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);

    // Close via the ×
    await page.locator("#criar-modal-close").click();
    await expect(page.locator("#criar-modal-overlay")).toHaveClass(/hidden/);
    expect(page.url()).toBe(initialUrl);
  });
});
