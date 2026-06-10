/**
 * Pipeline workflow — thinned (CO-302): auth gate + create + status progression.
 * Bulk operations, cross-view consistency, dashboard numbers, Conteúdo tab
 * moved to component tests or covered by other spec files.
 */

import { test, expect } from "./fixtures";
import {
  navigateTo,
  selectProject,
  waitForBoard,
  createTask,
} from "./helpers";

test.describe("Auth: unauthenticated writes rejected", () => {
  test("POST tasks without session returns 401", async ({
    request,  // unauthenticated plain context — no uat-login cookie
    seedProject,
  }) => {
    // `request` carries no session; `seedProject` uses apiContext internally.
    const res = await request.post(
      `/api/projects/${seedProject.key}/tasks`,
      { data: { title: "blocked" } },
    );
    expect(res.status()).toBe(401);
  });
});

test.describe("Create", () => {
  test("new task lands in To Do column", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, { title: "New todo task" });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const todoCol = page.locator('.kanban-column[data-status="todo"] .task-card');
    await expect(todoCol.filter({ hasText: "New todo task" })).toBeVisible();
    expect(task.id).toBeGreaterThan(0);
  });
});

test.describe("Status progression", () => {
  test("todo → in_progress moves card to correct column", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    test.skip((page.viewportSize()?.width ?? 1280) <= 640,
      "single-column mobile board (CO-358) — cross-column visibility is a desktop assertion");
    const task = await createTask(apiContext, seedProject.key, { title: "Progress task" });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
    await page.locator("#task-status").selectOption("in_progress");
    await page.locator("#task-form").dispatchEvent("submit");
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    const inProgressCol = page.locator(
      '.kanban-column[data-status="in_progress"] .task-card',
    );
    await expect(inProgressCol.filter({ hasText: "Progress task" })).toBeVisible();
  });
});

test.describe("Archive", () => {
  test("archived task is hidden by default", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, { title: "Archive task" });

    // Archive via PUT (server registers PUT /projects/{key}/tasks/{id}, not PATCH)
    const archiveRes = await apiContext.put(
      `/api/projects/${seedProject.key}/tasks/${task.id}`,
      { data: { title: task.title, status: "done", archived: true } },
    );
    expect([200, 204]).toContain(archiveRes.status());

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    await expect(
      page.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toHaveCount(0);
  });
});
