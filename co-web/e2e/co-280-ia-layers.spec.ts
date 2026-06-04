/**
 * CO-280 — Universe vs sub-universe vs deployable-unit: IA layers spec.
 *
 * Covers all three IA layers introduced by CO-280:
 *   Layer 1 — Platform context: breadcrumbs (CO › Universe › Sub-universe › Project)
 *   Layer 2 — Content universes / sub-universes: sidebar tree with active highlighting
 *   Layer 3 — Dev/operator tools: sidebar Tools section, separate from end-user nav
 *
 * Also verifies the co_dev_ship audit decision: the button is absent from the sidebar.
 */
import { test, expect } from "./fixtures";
import { navigateTo } from "./helpers";

test.describe("CO-280: IA layers — sidebar, breadcrumbs, tools", () => {
  // ── Layer 3: Tools section ────────────────────────────────────────────────────

  test("L3 — Tools section is visible and distinct from project nav", async ({
    page,
  }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar hidden on mobile");

    await navigateTo(page, "/");

    // Tools section must be rendered
    const toolsSection = page.locator("#sidebar-tools-section");
    await expect(toolsSection).toBeVisible();

    // Must contain at least one tool item (Phase 1: deployments + changelog)
    const toolItems = toolsSection.locator(".sidebar-tool-item");
    expect(await toolItems.count()).toBeGreaterThanOrEqual(1);

    // Critical IA invariant: tool items must NOT appear inside #project-list
    // (the user-reported symptom was dev affordances mixed with project nav)
    const toolsInProjectList = page.locator("#project-list .sidebar-tool-item");
    expect(await toolsInProjectList.count()).toBe(0);
  });

  test("L3 — co_dev_ship is not rendered as a button or link", async ({
    page,
  }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar hidden on mobile");

    await navigateTo(page, "/");

    await expect(page.locator('[data-tool="co_dev_ship"]')).toHaveCount(0);
    await expect(page.getByText("co_dev_ship")).toHaveCount(0);
  });

  // ── Layer 2: Universe sidebar tree ───────────────────────────────────────────

  test("L2 — Sidebar 'This universe' section contains universe and project nav", async ({
    page,
  }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar hidden on mobile");

    await navigateTo(page, "/");

    const thisUniverseSection = page.locator(".sidebar-this-universe");
    await expect(thisUniverseSection).toBeVisible();
    await expect(thisUniverseSection.locator("#project-list")).toBeVisible();
  });

  test("L2 — Active universe item is highlighted in the sidebar tree", async ({
    page,
  }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar hidden on mobile");

    // Authenticate so we see the full authenticated sidebar
    await page.request.post("/api/v1/auth/uat-login", {
      data: { email: "yuri@uat.local", password: "uat" },
    });

    await navigateTo(page, "/e2e-test");

    // The active universe item must carry the .active class
    const activeItem = page.locator(
      ".sidebar-universe-item.active[data-universe]",
    );
    await expect(activeItem.first()).toBeVisible();
  });

  test("L2 — Sub-universe items are indented relative to their parent", async ({
    page,
    apiContext,
  }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar hidden on mobile");

    await page.request.post("/api/v1/auth/uat-login", {
      data: { email: "yuri@uat.local", password: "uat" },
    });

    // Find any child universe in the DB
    const meRes = await apiContext.get("/api/v1/me/universes");
    const meData = await meRes.json();
    const allUniverses: { key: string; parent_key?: string }[] = [
      ...(meData.owned || []),
      ...(meData.member || []),
      ...(meData.subscribed || []),
    ];
    const child = allUniverses.find((u) => u.parent_key);

    if (!child) {
      test.skip(true, "No sub-universe in test DB — needs CO-277 seed data");
      return;
    }

    // Navigate to the parent universe — child should appear indented in the tree
    await navigateTo(page, `/${child.parent_key}`);

    const childItem = page.locator(
      `.sidebar-universe-item[data-universe="${child.key}"]`,
    );
    await expect(childItem).toBeVisible();

    // Indentation is applied via inline padding-left > 12px (base is 12px for depth 0)
    const paddingLeft = await childItem.evaluate(
      (el) => parseInt(window.getComputedStyle(el).paddingLeft, 10),
    );
    expect(paddingLeft).toBeGreaterThan(12);
  });

  // ── Layer 1: Platform context (breadcrumbs) ───────────────────────────────────

  test("L1 — Breadcrumbs element exists in the page", async ({ page }) => {
    await navigateTo(page, "/");
    await expect(page.locator("#breadcrumbs")).toBeAttached();
  });

  test("L1 — Breadcrumbs are hidden on the template root view", async ({
    page,
  }) => {
    await navigateTo(page, "/");
    // On template, breadcrumbs provide no useful context — they should be hidden
    await expect(page.locator("#breadcrumbs")).toHaveClass(/hidden/);
  });

  test("L1 — Breadcrumbs show platform and universe when viewing a named universe", async ({
    page,
  }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 480), "Breadcrumbs hidden on narrow mobile");

    await page.request.post("/api/v1/auth/uat-login", {
      data: { email: "yuri@uat.local", password: "uat" },
    });

    await navigateTo(page, "/e2e-test");

    const bc = page.locator("#breadcrumbs");
    await expect(bc).not.toHaveClass(/hidden/);

    // Platform "CO" crumb must always be present as a link
    await expect(bc.locator(".bc-link", { hasText: /^CO$/ })).toBeVisible();

    // Universe name crumb ("E2E Test") must be present
    await expect(
      bc.locator(".bc-item").filter({ hasText: /E2E Test/i }),
    ).toBeVisible();
  });

  test("L1 — Breadcrumbs include project name when a project is selected", async ({
    page,
    seedProject,
  }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 480), "Breadcrumbs hidden on narrow mobile");

    await navigateTo(page, "/");

    // Wait for sidebar to show the seeded project
    const projectItem = page.locator(
      `#project-list [data-key="${seedProject.key}"]`,
    );
    await expect(projectItem).toBeVisible({ timeout: 10_000 });
    await projectItem.click();

    // Default project view since CO-304 is `.conteudo-view`; older views
    // (.kanban-column, .universe-home) may also be reached depending on state.
    await page.waitForSelector(
      "#content.conteudo-view, .kanban-column, .universe-home",
      { timeout: 10_000 },
    );

    const bc = page.locator("#breadcrumbs");
    await expect(bc).not.toHaveClass(/hidden/);

    // Project name should be in the breadcrumb as the current-page crumb
    await expect(
      bc.locator(".bc-current", { hasText: seedProject.name }),
    ).toBeVisible();
  });

  test("L1 — Breadcrumbs show sub-universe trail when parent_key is set", async ({
    page,
    apiContext,
  }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 480), "Breadcrumbs hidden on narrow mobile");

    await page.request.post("/api/v1/auth/uat-login", {
      data: { email: "yuri@uat.local", password: "uat" },
    });

    // Check if yuri has any sub-universe
    const meRes = await apiContext.get("/api/v1/me/universes");
    const meData = await meRes.json();
    const allUniverses: {
      key: string;
      name: string;
      parent_key?: string;
    }[] = [
      ...(meData.owned || []),
      ...(meData.member || []),
      ...(meData.subscribed || []),
    ];
    const subUniverse = allUniverses.find((u) => u.parent_key);

    if (!subUniverse) {
      test.skip(
        true,
        "No sub-universe in test DB — needs CO-277 seed data (yggdrasil/shandara etc.)",
      );
      return;
    }

    await navigateTo(page, `/${subUniverse.key}`);

    const bc = page.locator("#breadcrumbs");
    await expect(bc).not.toHaveClass(/hidden/);

    // Should have at least 3 items: CO + parent + sub-universe
    const items = bc.locator(".bc-item");
    const count = await items.count();
    expect(count).toBeGreaterThanOrEqual(3);

    // Sub-universe name is the last (current) crumb
    await expect(
      bc.locator(".bc-current", { hasText: subUniverse.name }),
    ).toBeVisible();
  });

  // ── No regressions ────────────────────────────────────────────────────────────

  test("no-regression — board navigation still works after CO-280 changes", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");

    const projectItem = page.locator(
      `#project-list [data-key="${seedProject.key}"]`,
    );
    await expect(projectItem).toBeVisible({ timeout: 10_000 });
    await projectItem.click();

    // Project view must render (not crash). Default since CO-304 is conteudo.
    await page.waitForSelector(
      "#content.conteudo-view, .kanban-column, .universe-home",
      { timeout: 10_000 },
    );
  });
});
