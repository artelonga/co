/**
 * Board UX — thinned (CO-302): core viewing experience only.
 * Sidebar, view-switching, and keyboard shortcuts moved to component tests.
 */

import { test, expect } from "./fixtures";
import { navigateTo, selectProject, waitForBoard } from "./helpers";

test.describe("Empty states", () => {
  test("shows empty-state prompt before any project is chosen", async ({ page }) => {
    await navigateTo(page, "/");
    await expect(page.locator(".empty-state")).toBeVisible();
  });
});

test.describe("View tabs", () => {
  test("all 6 view tabs are visible after selecting a project", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);
    const tabs = page.locator("#view-tabs .view-tab");
    await expect(tabs).toHaveCount(6);
  });
});

test.describe("Mobile hamburger", () => {
  test("hamburger button is visible on narrow viewport", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await navigateTo(page, "/");
    await expect(page.locator("#hamburger-btn")).toBeVisible();
  });
});
