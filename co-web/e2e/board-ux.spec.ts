/**
 * Board UX — thinned (CO-302): core viewing experience only.
 * Sidebar, view-switching, and keyboard shortcuts moved to component tests.
 */

import { test, expect } from "./fixtures";
import { navigateTo, selectProject, waitForBoard } from "./helpers";

test.describe("Empty states", () => {
  test("shows empty-state prompt before any project is chosen", async ({ page }) => {
    await navigateTo(page, "/");
    // Use data-testid to target the specific "no project selected" element.
    // Class .empty-state is reused by loading placeholders across views and
    // would match 9+ elements — always use data-testid here.
    await expect(page.locator('[data-testid="no-project-selected"]')).toBeVisible();
  });
});

test.describe("View tabs", () => {
  test("all view tabs are visible after selecting a project", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);
    // Count only the tabs actually shown — the CO-368 Scrum tab is present in
    // the DOM but `hidden` until a universe's `_scrum.yaml` enables it, so it
    // must not count toward the "visible tabs" assertion.
    const tabs = page.locator("#view-tabs .view-tab:not(.hidden)");
    // 8 tabs: conteudo, kanban, table, calendar, timeline, dashboard, changelog, workspace (CO-352)
    await expect(tabs).toHaveCount(8);
  });
});

test.describe("Mobile hamburger", () => {
  test("hamburger button is visible on narrow viewport", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await navigateTo(page, "/");
    await expect(page.locator("#hamburger-btn")).toBeVisible();
  });
});
