/**
 * CO-346: /co board — anonymous visitor sees content.
 *
 * Root cause: `seed_co_universe_tasks` returned early (source dir absent)
 * before upserting the CO project entry, so `bootAppForUniverse` found zero
 * projects, never called `selectProject`, and rendered an empty board.
 * `list_dev_tasks` also only searched `work/` prefix, missing tasks at the
 * `public/` prefix written by the boot-time seed (CO-262).
 *
 * Fix: CO project upsert now runs before the source-dir guard.
 * `list_dev_tasks` also searches `public/` prefix entries.
 */

import { test, expect } from "./fixtures";
import { navigateTo, loginAsAdmin } from "./helpers";

test.describe("CO-346: /co board visibility", () => {
  test("anonymous visitor to /co sees the kanban board with at least one project", async ({
    page,
  }) => {
    // Default `page` has no session cookie — truly anonymous.
    await navigateTo(page, "/co");

    // Kanban columns are injected once bootAppForUniverse selects a project.
    await page.waitForSelector(".kanban-column", { timeout: 10_000 });

    // The CO project (or any project) must appear in the sidebar project list.
    const projectItems = page.locator("#project-list .sidebar-item-key");
    await expect(projectItems.first()).toBeVisible({ timeout: 5_000 });
  });

  test("anonymous API: /co returns projects (not an empty array)", async ({
    anonContext,
  }) => {
    const res = await anonContext.get("/api/v1/universes/co/projects");
    expect(res.status()).toBe(200);
    const projects = await res.json();
    expect(Array.isArray(projects)).toBe(true);
    expect(projects.length).toBeGreaterThan(0);
  });

  test("logged-in user navigating to /co stays on /co — no auto-bounce", async ({
    page,
  }) => {
    await loginAsAdmin(page);
    await navigateTo(page, "/co");

    // URL must not have been rewritten to the user's own universe.
    expect(page.url()).toContain("/co");
    // Board must render (not stuck loading or redirected away).
    await page.waitForSelector(".kanban-column", { timeout: 10_000 });
  });
});
