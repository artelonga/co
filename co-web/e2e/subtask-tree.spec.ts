/**
 * Subtask tree — thinned (CO-302): one test per view.
 * localStorage persistence and tree-line details moved to component tests.
 */

import { test, expect } from "./fixtures";
import {
  navigateTo,
  selectProject,
  waitForTable,
  createTask,
} from "./helpers";

test.describe("Subtask: kanban view", () => {
  test("parent card shows subtask toggle with count", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const parent = await createTask(apiContext, seedProject.key, { title: "Parent task" });
    await apiContext.post(`/api/projects/${seedProject.key}/tasks`, {
      data: { title: "Child task", parent: parent.id },
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    const toggle = page.locator(
      `.task-card[data-task-id="${parent.id}"] .subtask-toggle`,
    );
    await expect(toggle).toBeVisible();
    await expect(toggle).toContainText("1");
  });
});

test.describe("Subtask: table view", () => {
  test("child row has subtask-row class under the parent", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const parent = await createTask(apiContext, seedProject.key, { title: "Table parent" });
    await apiContext.post(`/api/projects/${seedProject.key}/tasks`, {
      data: { title: "Table child", parent: parent.id },
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    await page.locator('#view-tabs .view-tab[data-view="table"]').click();
    await waitForTable(page);

    await expect(page.locator(".subtask-row")).toBeVisible({ timeout: 5_000 });
  });
});
