/**
 * CO — Board UX: viewing experience
 *
 * Tests the visual and interactive experience of using the board without
 * touching mutation paths. Covers:
 *
 *   - Empty states (no project selected, no tasks)
 *   - Sidebar: project list, selection, active state
 *   - View switching: all 6 tabs, correct UI controls shown per view
 *   - Keyboard shortcuts: 1-6 for views, n for new task, / for search, Escape
 *   - Search/filter: typing narrows visible task cards
 *   - Task modal: open, close, field defaults
 *   - Timeline: zoom levels, prev/next/today nav, view-specific controls appear
 *   - Calendar: month grid renders, prev/next navigation
 *   - Dashboard: sections render when tasks exist
 *   - Conteúdo: sections render, no project required
 *   - Mini-calendar: appears on calendar view, hides on others
 *   - Mobile hamburger: sidebar opens/closes on narrow viewport
 */

import { test, expect } from "./fixtures";
import {
  navigateTo,
  selectProject,
  switchView,
  waitForBoard,
  waitForTable,
  waitForTimeline,
  createTask,
} from "./helpers";

// ─── Helpers ──────────────────────────────────────────────────────────────────

async function waitForCalendar(page: import("@playwright/test").Page) {
  await page.waitForSelector(".calendar", { state: "visible" });
}

async function waitForDashboard(page: import("@playwright/test").Page) {
  await page.waitForSelector(".dashboard, .dashboard-grid", { state: "visible" });
}

// ─── Empty states ─────────────────────────────────────────────────────────────

test.describe("Empty states", () => {
  test("shows 'Select a project' prompt before any project is chosen", async ({
    page,
  }) => {
    await navigateTo(page, "/");
    await expect(page.locator(".empty-state")).toBeVisible();
    await expect(page.locator(".empty-state")).toContainText(/select a project/i);
  });

  test("shows empty kanban columns when project has no tasks", async ({
    page,
    seedProject,
  }) => {
    // seedProject exists but createTask was never called — no tasks
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await waitForBoard(page);

    // 4 columns present, each with zero task cards
    const columns = page.locator(".kanban-column");
    await expect(columns).toHaveCount(4);

    const cards = page.locator(".task-card");
    await expect(cards).toHaveCount(0);
  });
});

// ─── Sidebar ──────────────────────────────────────────────────────────────────

test.describe("Sidebar: project list", () => {
  test("seeded project appears in the sidebar nav", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    // Create a second project so list has at least two entries
    const suffix = Math.random().toString(36).slice(2, 5).toUpperCase();
    await apiContext.post("/api/projects", {
      data: { name: `Extra ${suffix}`, key: `X${suffix}`, description: "" },
    });

    await navigateTo(page, "/");
    const entry = page.locator(
      `#project-list .sidebar-item-key:text-is("${seedProject.key}")`,
    );
    await expect(entry).toBeVisible();
  });

  test("clicking a project in the sidebar makes it active", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // The selected sidebar item should have an active/selected class or aria state
    const activeItem = page.locator(
      `#project-list .sidebar-item.active, #project-list .sidebar-item[aria-current]`,
    );
    await expect(activeItem).toHaveCount(1);
  });

  test("project name appears in the header after selection", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await expect(page.locator("#project-name")).toContainText(
      seedProject.name,
    );
  });
});

// ─── View switching ───────────────────────────────────────────────────────────

test.describe("View switching: tabs and view-specific controls", () => {
  test.beforeEach(async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar not available on mobile");
  });

  test("all 6 view tabs are visible after project selection", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    for (const view of ["kanban", "table", "timeline", "calendar", "dashboard", "conteudo"]) {
      await expect(
        page.locator(`#view-tabs .view-tab[data-view="${view}"]`),
      ).toBeVisible();
    }
  });

  test("switching to table renders the table container", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await createTask(apiContext, seedProject.key, { title: "Table row" });
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await switchView(page, "table");
    await waitForTable(page);
    await expect(page.locator(".table-container")).toBeVisible();
  });

  test("switching to timeline renders the timeline wrapper", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await createTask(apiContext, seedProject.key, {
      title: "Timeline task",
      due_date: "2026-08-01",
    });
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await switchView(page, "timeline");
    await waitForTimeline(page);
    await expect(page.locator(".timeline-wrapper")).toBeVisible();
  });

  test("switching to calendar renders the month grid", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await switchView(page, "calendar");
    await waitForCalendar(page);
    await expect(page.locator(".calendar")).toBeVisible();
  });

  test("switching to dashboard renders status bars", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await createTask(apiContext, seedProject.key, { title: "Dash task", status: "done" });
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await switchView(page, "dashboard");
    await waitForDashboard(page);
    await expect(page.locator(".dashboard-status-row").first()).toBeVisible();
  });

  test("zoom tabs and timeline nav appear only on timeline view", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Hidden on kanban
    await expect(page.locator("#zoom-tabs")).toHaveClass(/hidden/);
    await expect(page.locator("#timeline-nav")).toHaveClass(/hidden/);

    // Visible on timeline
    await switchView(page, "timeline");
    await waitForTimeline(page);
    await expect(page.locator("#zoom-tabs")).not.toHaveClass(/hidden/);
    await expect(page.locator("#timeline-nav")).not.toHaveClass(/hidden/);

    // Hidden again when switching away
    await switchView(page, "kanban");
    await waitForBoard(page);
    await expect(page.locator("#zoom-tabs")).toHaveClass(/hidden/);
  });

  test("active tab class moves to the selected view tab", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await switchView(page, "table");
    await expect(
      page.locator('#view-tabs .view-tab[data-view="table"]'),
    ).toHaveClass(/active/);
    await expect(
      page.locator('#view-tabs .view-tab[data-view="kanban"]'),
    ).not.toHaveClass(/active/);
  });
});

// ─── Keyboard shortcuts ───────────────────────────────────────────────────────

test.describe("Keyboard shortcuts", () => {
  test.beforeEach(async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar not available on mobile");
  });

  test("pressing 1 switches to kanban view", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "table");

    await page.keyboard.press("1");
    await waitForBoard(page);

    await expect(
      page.locator('#view-tabs .view-tab[data-view="kanban"]'),
    ).toHaveClass(/active/);
  });

  test("pressing 2 switches to table view", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.keyboard.press("2");
    await waitForTable(page);

    await expect(
      page.locator('#view-tabs .view-tab[data-view="table"]'),
    ).toHaveClass(/active/);
  });

  test("pressing 3 switches to timeline view", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.keyboard.press("3");
    await waitForTimeline(page);

    await expect(
      page.locator('#view-tabs .view-tab[data-view="timeline"]'),
    ).toHaveClass(/active/);
  });

  test("pressing 4 switches to calendar view", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.keyboard.press("4");
    await waitForCalendar(page);

    await expect(
      page.locator('#view-tabs .view-tab[data-view="calendar"]'),
    ).toHaveClass(/active/);
  });

  test("pressing 6 switches to Conteúdo view", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.keyboard.press("6");
    await expect(page.locator(".conteudo-grid")).toBeVisible();

    await expect(
      page.locator('#view-tabs .view-tab[data-view="conteudo"]'),
    ).toHaveClass(/active/);
  });

  test("pressing n opens the new task modal", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.keyboard.press("n");
    await expect(page.locator("#modal-overlay")).toBeVisible();
    await expect(page.locator("#modal-title")).toContainText(/new task/i);
  });

  test("pressing / focuses the search input", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.keyboard.press("/");
    await expect(page.locator("#search-input")).toBeFocused();
  });

  test("pressing Escape closes the task modal", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator("#btn-new-task").click();
    await expect(page.locator("#modal-overlay")).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(page.locator("#modal-overlay")).not.toBeVisible();
  });
});

// ─── Search / filter ──────────────────────────────────────────────────────────

test.describe("Search: filter narrows visible cards", () => {
  test.beforeEach(async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar not available on mobile");
  });

  test("typing a query hides non-matching task cards", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await Promise.all([
      createTask(apiContext, seedProject.key, { title: "Alpha feature" }),
      createTask(apiContext, seedProject.key, { title: "Beta fix" }),
      createTask(apiContext, seedProject.key, { title: "Gamma release" }),
    ]);

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    await page.locator("#search-input").fill("Beta");

    // Only the Beta card should remain visible
    const visibleCards = page.locator(".task-card:visible");
    await expect(visibleCards).toHaveCount(1);
    await expect(visibleCards.first()).toContainText("Beta fix");
  });

  test("clearing the search restores all cards", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await Promise.all([
      createTask(apiContext, seedProject.key, { title: "Task X" }),
      createTask(apiContext, seedProject.key, { title: "Task Y" }),
    ]);

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const searchInput = page.locator("#search-input");
    await searchInput.fill("Task X");
    await expect(page.locator(".task-card:visible")).toHaveCount(1);

    await searchInput.clear();
    const allCards = page.locator(".task-card");
    await expect(allCards).toHaveCount(2);
  });

  test("search is case-insensitive", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await createTask(apiContext, seedProject.key, { title: "CaseSensitivity Test" });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    await page.locator("#search-input").fill("casesensitivity");

    const cards = page.locator(".task-card:visible");
    await expect(cards).toHaveCount(1);
    await expect(cards.first()).toContainText("CaseSensitivity Test");
  });
});

// ─── New task modal ───────────────────────────────────────────────────────────

test.describe("New task modal: open and defaults", () => {
  test.beforeEach(async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar not available on mobile");
  });

  test("clicking '+ New Task' opens the modal", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator("#btn-new-task").click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
  });

  test("modal title reads 'New Task' on create", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator("#btn-new-task").click();
    await expect(page.locator("#modal-title")).toContainText(/new task/i);
  });

  test("status defaults to 'todo' in new task modal", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator("#btn-new-task").click();
    await expect(page.locator("#task-status")).toHaveValue("todo");
  });

  test("priority defaults to 'medium' in new task modal", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator("#btn-new-task").click();
    await expect(page.locator("#task-priority")).toHaveValue("medium");
  });

  test("clicking Cancel closes the modal without saving", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator("#btn-new-task").click();
    await page.locator("#task-title").fill("Should not be saved");
    await page.locator("#btn-cancel").click();

    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    // No new card should appear
    const cards = page.locator(".task-card");
    await expect(cards).toHaveCount(0);
  });

  test("clicking X closes the modal", async ({ page, seedProject }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator("#btn-new-task").click();
    await page.locator("#modal-close").click();

    await expect(page.locator("#modal-overlay")).not.toBeVisible();
  });

  test("clicking an existing task card opens modal with that task's title", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Existing card click",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
    await expect(page.locator("#task-title")).toHaveValue("Existing card click");
  });
});

// ─── Timeline controls ────────────────────────────────────────────────────────

test.describe("Timeline: zoom levels and navigation", () => {
  test.beforeEach(async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar not available on mobile");
  });

  test("zoom tabs show Week / Month / Quarter options", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "timeline");
    await waitForTimeline(page);

    await expect(page.locator('#zoom-tabs .view-tab[data-zoom="week"]')).toBeVisible();
    await expect(page.locator('#zoom-tabs .view-tab[data-zoom="month"]')).toBeVisible();
    await expect(page.locator('#zoom-tabs .view-tab[data-zoom="quarter"]')).toBeVisible();
  });

  test("clicking Week zoom updates the active zoom tab", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "timeline");
    await waitForTimeline(page);

    await page.locator('#zoom-tabs .view-tab[data-zoom="week"]').click();
    await expect(
      page.locator('#zoom-tabs .view-tab[data-zoom="week"]'),
    ).toHaveClass(/active/);
  });

  test("Today button re-renders the timeline without error", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "timeline");
    await waitForTimeline(page);

    await page.locator("#btn-today").click();
    await expect(page.locator(".timeline-wrapper")).toBeVisible();
  });

  test("← and → nav buttons shift the timeline viewport", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await createTask(apiContext, seedProject.key, {
      title: "Nav test task",
      due_date: "2026-09-15",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "timeline");
    await waitForTimeline(page);

    // Grab initial header text
    const header = page.locator(".timeline-header, .tl-header").first();
    const before = await header.textContent();

    await page.locator("#btn-next").click();
    await page.waitForTimeout(100); // allow re-render

    const after = await header.textContent();
    // Header content should differ after navigating
    expect(after).not.toBe(before);

    // Return to today
    await page.locator("#btn-today").click();
  });
});

// ─── Calendar navigation ──────────────────────────────────────────────────────

test.describe("Calendar: month grid and navigation", () => {
  test.beforeEach(async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar not available on mobile");
  });

  test("calendar month grid has 7-column header (day names)", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "calendar");
    await waitForCalendar(page);

    const dayHeaders = page.locator(
      ".calendar-day-header, .cal-day-name, .calendar th",
    );
    await expect(dayHeaders).toHaveCount(7);
  });

  test("calendar shows month and year in its header", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "calendar");
    await waitForCalendar(page);

    const calHeader = page.locator(
      ".calendar-header h2, .calendar-month-label, .cal-header",
    );
    await expect(calHeader.first()).toBeVisible();
    // Should contain a year (4 digits)
    await expect(calHeader.first()).toContainText(/20\d\d/);
  });

  test("clicking the next-month button changes the displayed month", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "calendar");
    await waitForCalendar(page);

    const monthLabel = page.locator(
      ".calendar-header h2, .calendar-month-label, .cal-header",
    ).first();
    const before = await monthLabel.textContent();

    const nextBtn = page.locator(
      ".calendar-nav-next, button[aria-label='Next month'], .btn-cal-next",
    ).first();
    await nextBtn.click();

    const after = await monthLabel.textContent();
    expect(after).not.toBe(before);
  });
});

// ─── Mini-calendar sidebar ────────────────────────────────────────────────────

test.describe("Mini-calendar: visibility by view", () => {
  test.beforeEach(async ({ page }) => {
    const vp = page.viewportSize();
    test.skip(!!(vp && vp.width <= 640), "Sidebar not available on mobile");
  });

  test("mini-calendar is visible on calendar view", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "calendar");
    await waitForCalendar(page);

    await expect(page.locator("#mini-calendar")).not.toHaveClass(/hidden/);
  });

  test("mini-calendar is hidden on kanban view", async ({
    page,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    // Kanban is the default — mini-calendar should be hidden
    await expect(page.locator("#mini-calendar")).toHaveClass(/hidden/);
  });
});

// ─── Mobile hamburger ─────────────────────────────────────────────────────────

test.describe("Mobile hamburger menu", () => {
  test.use({ viewport: { width: 390, height: 844 } }); // iPhone 14

  test("hamburger button is visible on narrow viewport", async ({ page }) => {
    await navigateTo(page, "/");
    await expect(page.locator("#hamburger-btn")).toBeVisible();
  });

  test("tapping hamburger opens the sidebar overlay", async ({ page }) => {
    await navigateTo(page, "/");
    await page.locator("#hamburger-btn").click();
    await expect(page.locator("#sidebar-overlay")).toHaveClass(/visible/);
  });

  test("tapping the overlay closes the sidebar", async ({ page }) => {
    await navigateTo(page, "/");
    await page.locator("#hamburger-btn").click();
    await expect(page.locator("#sidebar-overlay")).toHaveClass(/visible/);

    await page.locator("#sidebar-overlay").click();
    await expect(page.locator("#sidebar-overlay")).not.toHaveClass(/visible/);
  });
});
