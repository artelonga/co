/**
 * CO-2 — Subtask tree rendering with expand/collapse
 *
 * Covers all acceptance criteria:
 *  - Kanban: parent card shows expandable subtask list (click toggle)
 *  - Table: indented rows with tree connector lines
 *  - Timeline: subtasks grouped under parent swimlane, collapsible
 *  - Task modal: parent link + subtask list
 *  - Persist expand/collapse state in localStorage
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

// Skip all tests on mobile (<= 640px) — sidebar is hidden at that breakpoint
test.beforeEach(async ({ page }) => {
  const vp = page.viewportSize();
  test.skip(
    !!(vp && vp.width <= 640),
    "Sidebar not available on mobile viewport",
  );
});

// ─── Shared setup helper ──────────────────────────────────────────────────────

async function setupParentAndChild(
  apiContext: Parameters<typeof createTask>[0],
  projectKey: string,
) {
  const parent = await createTask(apiContext, projectKey, {
    title: "Parent task",
    status: "todo",
  });
  const child = await createTask(apiContext, projectKey, {
    title: "Child subtask",
    status: "todo",
    parent: parent.id,
  });
  return { parent, child };
}

// ─── Kanban ───────────────────────────────────────────────────────────────────

test.describe("Kanban: subtask expand/collapse", () => {
  test("parent card has a subtask toggle showing child count", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    const toggle = page.locator(".subtask-toggle").first();
    await expect(toggle).toBeVisible();
    await expect(toggle).toContainText("1 subtask");
  });

  test("clicking the toggle collapses the subtask list", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // List is expanded by default
    const list = page.locator(".subtask-list").first();
    await expect(list).toBeVisible();

    // Collapse
    await page.locator(".subtask-toggle").first().click();
    await expect(list).not.toBeVisible();
  });

  test("clicking again re-expands the subtask list", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    const toggle = page.locator(".subtask-toggle").first();
    const list = page.locator(".subtask-list").first();

    await toggle.click(); // collapse
    await toggle.click(); // expand
    await expect(list).toBeVisible();
  });

  test("child task does NOT appear as a standalone top-level card", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { child } = await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Top-level cards (direct children of .kanban-cards) should not include the child
    const standaloneChild = page.locator(
      `.kanban-cards > .task-card[data-task-id="${child.id}"]`,
    );
    await expect(standaloneChild).toHaveCount(0);
  });

  test("subtask item inside expanded list is visible with key and title", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { child } = await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    const item = page
      .locator(".subtask-list .subtask-item")
      .filter({ hasText: child.title });
    await expect(item).toBeVisible();
    await expect(item.locator(".subtask-item-key")).toContainText(child.key);
  });

  test("clicking a subtask item opens the task modal for that subtask", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { child } = await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page
      .locator(".subtask-list .subtask-item")
      .filter({ hasText: child.title })
      .click();

    const modal = page.locator("#modal-overlay");
    await expect(modal).toBeVisible();
    await expect(page.locator("#modal-title")).toContainText(child.key);
  });
});

// ─── Table ────────────────────────────────────────────────────────────────────

test.describe("Table: tree connector lines and indentation", () => {
  test("subtask row appears indented under its parent in the same group", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent, child } = await setupParentAndChild(
      apiContext,
      seedProject.key,
    );
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "table");
    await waitForTable(page);

    const parentRow = page.locator(`tr[data-task-id="${parent.id}"]`);
    const childRow = page.locator(`tr[data-task-id="${child.id}"]`);

    await expect(parentRow).toBeVisible();
    await expect(childRow).toBeVisible();
    await expect(childRow).toHaveClass(/subtask-row/);
  });

  test("tree connector └─ is shown in the child row", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { child } = await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "table");
    await waitForTable(page);

    const connector = page.locator(
      `tr[data-task-id="${child.id}"] .tree-line`,
    );
    await expect(connector).toBeVisible();
    await expect(connector).toContainText("└─");
  });

  test("parent row has a ▼ toggle button when it has children in the group", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent } = await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "table");
    await waitForTable(page);

    const toggleBtn = page.locator(
      `tr[data-task-id="${parent.id}"] .tree-toggle`,
    );
    await expect(toggleBtn).toBeVisible();
    await expect(toggleBtn).toContainText("▼");
  });

  test("clicking the tree toggle hides the child row", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent, child } = await setupParentAndChild(
      apiContext,
      seedProject.key,
    );
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "table");
    await waitForTable(page);

    const childRow = page.locator(`tr[data-task-id="${child.id}"]`);
    await expect(childRow).toBeVisible();

    const toggleBtn = page.locator(
      `tr[data-task-id="${parent.id}"] .tree-toggle`,
    );
    await toggleBtn.click();

    await expect(childRow).not.toBeVisible();
    await expect(toggleBtn).toContainText("▶");
  });

  test("clicking toggle again re-shows the child row", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent, child } = await setupParentAndChild(
      apiContext,
      seedProject.key,
    );
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "table");
    await waitForTable(page);

    const toggleBtn = page.locator(
      `tr[data-task-id="${parent.id}"] .tree-toggle`,
    );
    await toggleBtn.click(); // collapse
    await toggleBtn.click(); // expand

    const childRow = page.locator(`tr[data-task-id="${child.id}"]`);
    await expect(childRow).toBeVisible();
  });
});

// ─── Timeline ─────────────────────────────────────────────────────────────────

test.describe("Timeline: subtasks grouped under parent swimlane", () => {
  test("parent row has a subtree toggle when it has subtasks in the same swimlane", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "timeline");
    await waitForTimeline(page);

    const toggle = page.locator(".timeline-subtree-toggle").first();
    await expect(toggle).toBeVisible();
  });

  test("subtask row is visible under its parent row when expanded", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { child } = await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "timeline");
    await waitForTimeline(page);

    const subRow = page.locator(
      `.timeline-subtask-row[data-task-id="${child.id}"]`,
    );
    await expect(subRow).toBeVisible();
  });

  test("clicking the toggle hides the subtask row", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent, child } = await setupParentAndChild(
      apiContext,
      seedProject.key,
    );
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "timeline");
    await waitForTimeline(page);

    const subRow = page.locator(
      `.timeline-subtask-row[data-task-id="${child.id}"]`,
    );
    await expect(subRow).toBeVisible();

    const toggle = page.locator(
      `.timeline-subtree-toggle[data-task-id="${parent.id}"]`,
    );
    await toggle.click();

    await expect(subRow).not.toBeVisible();
  });
});

// ─── Modal ────────────────────────────────────────────────────────────────────

test.describe("Modal: parent link and subtask list", () => {
  test("opening a subtask shows the parent task as a link", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent, child } = await setupParentAndChild(
      apiContext,
      seedProject.key,
    );
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Open child task modal via API key displayed on card inside subtask list
    // The child is inside the subtask list of the parent card
    await page
      .locator(".subtask-list .subtask-item")
      .filter({ hasText: child.title })
      .click();

    const hierInfo = page.locator("#task-hierarchy-info");
    await expect(hierInfo).toBeVisible();

    const parentLink = hierInfo.locator(".hierarchy-link");
    await expect(parentLink).toBeVisible();
    await expect(parentLink).toContainText(parent.title);
  });

  test("clicking the parent link navigates to the parent modal", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent, child } = await setupParentAndChild(
      apiContext,
      seedProject.key,
    );
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Open child
    await page
      .locator(".subtask-list .subtask-item")
      .filter({ hasText: child.title })
      .click();

    const parentLink = page.locator("#task-hierarchy-info .hierarchy-link");
    await parentLink.click();

    await expect(page.locator("#modal-title")).toContainText(parent.key);
  });

  test("opening a parent task shows its subtask list with status badges", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent, child } = await setupParentAndChild(
      apiContext,
      seedProject.key,
    );
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Open parent card directly
    await page.locator(`.task-card[data-task-id="${parent.id}"]`).click();

    const hierInfo = page.locator("#task-hierarchy-info");
    await expect(hierInfo).toBeVisible();

    const subtaskItem = hierInfo
      .locator(".hierarchy-subtask-item")
      .filter({ hasText: child.title });
    await expect(subtaskItem).toBeVisible();

    // Status badge should be present
    await expect(subtaskItem.locator(".hierarchy-subtask-status")).toBeVisible();
  });

  test("clicking a subtask in the modal list navigates to that subtask", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent, child } = await setupParentAndChild(
      apiContext,
      seedProject.key,
    );
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator(`.task-card[data-task-id="${parent.id}"]`).click();

    const subtaskItem = page
      .locator("#task-hierarchy-info .hierarchy-subtask-item")
      .filter({ hasText: child.title });
    await subtaskItem.click();

    await expect(page.locator("#modal-title")).toContainText(child.key);
  });

  test("opening a task with no parent and no subtasks shows no hierarchy section", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const lone = await createTask(apiContext, seedProject.key, {
      title: "Lone task",
    });
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator(`.task-card[data-task-id="${lone.id}"]`).click();

    await expect(page.locator("#task-hierarchy-info")).toHaveCount(0);
  });
});

// ─── localStorage persistence ─────────────────────────────────────────────────

test.describe("localStorage: expand/collapse state persists across reload", () => {
  test("collapsed subtask list stays collapsed after page reload", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Collapse
    await page.locator(".subtask-toggle").first().click();

    // Verify collapsed
    await expect(page.locator(".subtask-list").first()).not.toBeVisible();

    // Reload and re-select project
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Should still be collapsed
    await expect(page.locator(".subtask-list").first()).not.toBeVisible();
  });

  test("localStorage key co_subtree_<KEY> holds the collapsed task ID", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const { parent } = await setupParentAndChild(apiContext, seedProject.key);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Collapse the parent
    await page.locator(".subtask-toggle").first().click();

    const stored = await page.evaluate(
      (key) => localStorage.getItem(`co_subtree_${key}`),
      seedProject.key,
    );
    expect(stored).not.toBeNull();
    const ids: number[] = JSON.parse(stored!);
    expect(ids).toContain(parent.id);
  });
});
