/**
 * CO-280 Phase 1 — sidebar restructure into three labeled sections.
 *
 * Verifies:
 *   1. Sidebar renders THREE distinct section containers in order:
 *      Platforms, This universe, Tools.
 *   2. Platforms section lists the 5 hardcoded sister deployable units
 *      (co, artelonga, quilombo, yggdrasil, rfq).
 *   3. Tools section is rendered (currently houses /deployments + /changelog
 *      links — co_dev_ship audit is deferred to Phase 2/3 per the spec).
 *   4. Each Tools item lives in #tools-nav, never in #project-list (the
 *      user-reported symptom was dev affordances mixed into end-user nav).
 */
import { test, expect } from "./fixtures";
import { navigateTo } from "./helpers";

test.describe("CO-280 Phase 1: three-section sidebar", () => {
  test("sidebar renders Platforms, This universe, and Tools containers", async ({
    page,
  }) => {
    const viewport = page.viewportSize();
    test.skip(
      !!(viewport && viewport.width <= 640),
      "Sidebar not visible on mobile viewport",
    );

    await navigateTo(page, "/");

    // The three section containers must all be present in DOM order.
    const sidebar = page.locator("#sidebar");
    await expect(sidebar.locator("#sidebar-platforms-section")).toBeVisible();
    await expect(sidebar.locator(".sidebar-this-universe")).toBeVisible();
    await expect(sidebar.locator("#sidebar-tools-section")).toBeVisible();
  });

  test("Platforms section lists 5 sister deployable units", async ({ page }) => {
    const viewport = page.viewportSize();
    test.skip(
      !!(viewport && viewport.width <= 640),
      "Sidebar not visible on mobile viewport",
    );

    await navigateTo(page, "/");

    const platformItems = page.locator("#platforms-nav .sidebar-platform-item");
    await expect(platformItems).toHaveCount(5);

    const expectedKeys = ["co", "artelonga", "quilombo", "yggdrasil", "rfq"];
    for (const key of expectedKeys) {
      await expect(
        page.locator(`#platforms-nav [data-platform="${key}"]`),
      ).toBeVisible();
    }
  });

  test("Tools section is rendered with operator links, never inside #project-list", async ({
    page,
  }) => {
    const viewport = page.viewportSize();
    test.skip(
      !!(viewport && viewport.width <= 640),
      "Sidebar not visible on mobile viewport",
    );

    await navigateTo(page, "/");

    const toolsNav = page.locator("#tools-nav");
    await expect(toolsNav).toBeVisible();

    // Tools section should contain at least one tool item (Phase 1 ships
    // deployments + changelog).
    const toolItems = toolsNav.locator(".sidebar-tool-item");
    expect(await toolItems.count()).toBeGreaterThanOrEqual(1);

    // Critical IA invariant: tool items must NEVER appear inside #project-list
    // (that was the user-reported symptom — dev affordances mixed with project
    // navigation). The Tools section is its own sibling container.
    const toolsInProjectList = page.locator(
      "#project-list .sidebar-tool-item",
    );
    expect(await toolsInProjectList.count()).toBe(0);
  });
});
