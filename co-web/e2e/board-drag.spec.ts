/**
 * CO-33 — Board CRUD: drag-and-drop between columns
 *
 * Covers the full Board CRUD sequence:
 *   - create project → create task → drag between columns → edit task → delete task
 *
 * Drag tests use Playwright's built-in dragTo() API which dispatches
 * dragstart / dragenter / dragover / drop events matching the app's
 * ondragstart / ondragover / ondrop handlers.
 */

import { test, expect } from "./fixtures";
import {
  navigateTo,
  selectProject,
  waitForBoard,
  createTask,
} from "./helpers";

// Skip all tests on mobile — sidebar is hidden at ≤ 640 px
test.beforeEach(async ({ page }) => {
  const vp = page.viewportSize();
  test.skip(
    !!(vp && vp.width <= 640),
    "Sidebar not available on mobile viewport",
  );
});

// ─── Drag between columns ─────────────────────────────────────────────────────

test.describe("Drag: move task card between kanban columns", () => {
  test("drag from 'To Do' to 'In Progress' updates the task status via API", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Drag test task",
      status: "todo",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    await expect(card).toBeVisible();

    // Target: the In Progress column drop zone
    const inProgressCol = page
      .locator('.kanban-column[data-status="in_progress"]')
      .or(page.locator(".kanban-column").filter({ hasText: "In Progress" }));
    await expect(inProgressCol).toBeVisible();

    await card.dragTo(inProgressCol);

    // Card should now be inside the In Progress column
    await expect(
      inProgressCol.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible({ timeout: 5_000 });
  });

  test("drag from 'In Progress' to 'Done' column", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Done drag task",
      status: "in_progress",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    await expect(card).toBeVisible();

    const doneCol = page
      .locator('.kanban-column[data-status="done"]')
      .or(page.locator(".kanban-column").filter({ hasText: "Done" }));

    await card.dragTo(doneCol);

    await expect(
      doneCol.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible({ timeout: 5_000 });
  });
});

// ─── Full CRUD sequence ───────────────────────────────────────────────────────

test.describe("Full CRUD: create → drag → edit → delete", () => {
  test("create project task, drag to In Progress, edit title, then delete", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    // Step 1: Create task via API (represents "create project task")
    const task = await createTask(apiContext, seedProject.key, {
      title: "CRUD flow task",
      status: "todo",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    await expect(card).toBeVisible();

    // Step 2: Drag to "In Progress"
    const inProgressCol = page
      .locator('.kanban-column[data-status="in_progress"]')
      .or(page.locator(".kanban-column").filter({ hasText: "In Progress" }));
    await card.dragTo(inProgressCol);
    await expect(
      inProgressCol.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible({ timeout: 5_000 });

    // Step 3: Edit task title via modal
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
    await page.locator("#task-title").fill("CRUD flow task — edited");
    await page.locator("#task-form").dispatchEvent("submit");
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    await expect(
      page.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toContainText("CRUD flow task — edited");

    // Step 4: Delete task via modal
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
    page.on("dialog", (d) => d.accept());
    await page.locator("#btn-delete").click();
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    // Task should be gone from the board
    await expect(
      page.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toHaveCount(0);

    // API confirms deletion
    const getRes = await apiContext.get(
      `/api/projects/${seedProject.key}/tasks/${task.id}`,
    );
    expect(getRes.status()).toBe(404);
  });
});
