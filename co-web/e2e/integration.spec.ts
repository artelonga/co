/**
 * Integration — thinned (CO-302): auth API contract + core CRUD round-trip.
 * Login UI flow, session injection, sign-out, activity feed, dashboard
 * numbers moved to component tests or covered by auth.spec.ts.
 */

import { test, expect } from "./fixtures";
import {
  navigateTo,
  selectProject,
  waitForBoard,
  createTask,
} from "./helpers";

// ─── Auth API ─────────────────────────────────────────────────────────────────

test.describe("Auth API contract", () => {
  test("POST /v1/auth/login returns 200 (never reveals user existence)", async ({
    request,
  }) => {
    const res = await request.post("/api/v1/auth/login", {
      data: { email: "nobody@notregistered.example.com" },
    });
    expect(res.status()).toBe(200);
  });

  test("GET /v1/auth/me without session returns 401", async ({ request }) => {
    const res = await request.get("/api/v1/auth/me");
    expect(res.status()).toBe(401);
  });
});

// ─── CRUD round-trip ──────────────────────────────────────────────────────────

test.describe("CRUD round-trip", () => {
  test("created task appears on the board", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, { title: "CRUD task" });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    await expect(page.locator(`.task-card[data-task-id="${task.id}"]`)).toBeVisible();
  });

  test("status change moves task to a different column", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, { title: "Status task" });

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
    await expect(inProgressCol.filter({ hasText: "Status task" })).toBeVisible();
  });

  test("deleted task disappears from the board", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, { title: "Delete task" });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    // Delete via API and verify it is gone from the DOM on reload
    await apiContext.delete(`/api/projects/${seedProject.key}/tasks/${task.id}`);
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    await expect(
      page.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toHaveCount(0);
  });
});
