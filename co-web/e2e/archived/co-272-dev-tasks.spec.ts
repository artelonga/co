/**
 * CO-272 — kanban shows entries-as-tasks from work/ (dev-board dogfooding gap)
 *
 * Verifies:
 * 1. GET /dev-tasks returns task-shaped entries from the work/ folder.
 * 2. Anonymous visit to /co shows ≥ 5 kanban cards in the "done" column.
 */

import { test, expect } from "./fixtures";

const SLUG = "co";

test.describe("CO-272: kanban shows dev-task entries", () => {
  test("GET /api/v1/universes/co/dev-tasks returns tasks with expected fields", async ({
    apiContext,
  }) => {
    const res = await apiContext.get(
      `/api/v1/universes/${SLUG}/dev-tasks?limit=10`,
    );
    expect(res.status()).toBe(200);

    const body = await res.json();
    expect(body.total).toBeGreaterThan(0);
    expect(Array.isArray(body.tasks)).toBe(true);
    expect(body.tasks.length).toBeGreaterThan(0);

    const task = body.tasks[0];
    expect(typeof task.key).toBe("string");
    expect(task.key.length).toBeGreaterThan(0);
    expect(typeof task.title).toBe("string");
    expect(typeof task.status).toBe("string");
  });

  test("anonymous visit to /co shows ≥ 5 kanban cards in the done column", async ({
    page,
  }) => {
    await page.goto(`/${SLUG}`, { waitUntil: "networkidle" });

    const doneColumn = page.locator('.kanban-column[data-status="done"]');
    await expect(doneColumn).toBeVisible({ timeout: 15_000 });

    const doneCards = doneColumn.locator(".task-card");
    const count = await doneCards.count();
    expect(count).toBeGreaterThanOrEqual(5);
  });
});
