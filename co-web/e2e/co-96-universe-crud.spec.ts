/**
 * CO-96 — Universe CRUD UI
 *
 * Covers the SPA surfaces added for the universe lifecycle:
 *   - Sidebar "+ New universe" button opens the create modal
 *   - Inline key validation (format + disabled submit on a bad key)
 *   - Right-click a sidebar universe → context menu with the CRUD actions
 *   - Delete confirmation modal requires typing the exact key
 *   - Trash / recovery view opens from the sidebar
 *
 * These exercise UI wiring only — destructive API calls (actual delete) are
 * not confirmed here so the suite stays deterministic against shared data.
 */

import { test, expect } from "./fixtures";

test.describe("CO-96: Universe CRUD UI", () => {
  test("sidebar exposes the new-universe and trash buttons", async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar hidden on mobile");
    await page.goto("/", { waitUntil: "networkidle" });
    await expect(page.locator("#btn-sidebar-new-universe")).toBeVisible();
    await expect(page.locator("#btn-sidebar-trash")).toBeVisible();
  });

  test("sidebar '+ New universe' button opens the create modal", async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar hidden on mobile");
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#btn-sidebar-new-universe").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);
    await expect(page.locator("#criar-name")).toBeVisible();
  });

  test("inline validation: an invalid key disables submit and shows an error", async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar hidden on mobile");
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#btn-sidebar-new-universe").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);

    // A single character is below the 2-char minimum.
    await page.locator("#criar-slug").fill("x");
    await expect(page.locator("#criar-error")).not.toHaveClass(/hidden/);
    await expect(page.locator("#criar-submit")).toBeDisabled();

    // A well-formed key clears the error and re-enables submit.
    await page.locator("#criar-slug").fill(`e2e-valid-${Date.now()}`);
    await expect(page.locator("#criar-submit")).toBeEnabled();
  });

  test("right-click a universe item opens the context menu with CRUD actions", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    const item = page.locator(".sidebar-universe-item").first();
    // Skip gracefully if the anonymous sidebar has no universe rows.
    if ((await item.count()) === 0) test.skip(true, "no universe items in sidebar");

    await item.click({ button: "right" });
    const menu = page.locator("#universe-context-menu");
    await expect(menu).not.toHaveClass(/hidden/);
    await expect(menu.locator('[data-action="rename"]')).toBeVisible();
    await expect(menu.locator('[data-action="duplicate"]')).toBeVisible();
    await expect(menu.locator('[data-action="visibility"]').first()).toBeVisible();
  });

  test("delete confirmation requires typing the exact key", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    // Drive the modal directly through the exposed action helper so the test
    // doesn't depend on a particular universe being deletable.
    await page.evaluate(async () => {
      const mod = await import("/modules/sidebar/universe-actions.js");
      mod.openDeleteUniverseModal("demo-key", "Demo");
    });

    const overlay = page.locator("#delete-universe-overlay");
    await expect(overlay).not.toHaveClass(/hidden/);
    const confirm = page.locator("#delete-universe-confirm");
    await expect(confirm).toBeDisabled();

    await page.locator("#delete-universe-input").fill("wrong");
    await expect(confirm).toBeDisabled();

    await page.locator("#delete-universe-input").fill("demo-key");
    await expect(confirm).toBeEnabled();
  });

  test("trash view opens from the sidebar", async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar hidden on mobile");
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#btn-sidebar-trash").click();
    await expect(page.locator("#trash-overlay")).not.toHaveClass(/hidden/);
  });
});
