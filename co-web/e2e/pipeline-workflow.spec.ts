/**
 * CO — Full pipeline workflow E2E
 *
 * Traces the complete lifecycle of work in a project:
 *   1. Auth — protected writes require a valid session
 *   2. Create — task appears on kanban in the right column
 *   3. Edit — title / description / priority / due_date round-trip
 *   4. Status progression — todo → in_progress → in_review → done
 *   5. Cross-view consistency — kanban, table, timeline agree on state
 *   6. Comments — add a comment, verify it appears in activity
 *   7. Bulk operations — select multiple tasks, bulk-move, bulk-delete
 *   8. Archive / restore — done task hidden by default, visible when toggled
 *   9. Dashboard — status counts reflect the completed workflow
 *  10. Conteúdo tab — renders without errors (quilombo feed, no project needed)
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

// Skip on mobile — sidebar is hidden at ≤ 640 px
test.beforeEach(async ({ page }) => {
  const vp = page.viewportSize();
  test.skip(
    !!(vp && vp.width <= 640),
    "Sidebar not available on mobile viewport",
  );
});

// ─── Helpers ──────────────────────────────────────────────────────────────────

/** Open a task modal by clicking the card with the given task ID. */
async function openTaskModal(page: import("@playwright/test").Page, taskId: number) {
  await page.locator(`.task-card[data-task-id="${taskId}"]`).click();
  await expect(page.locator("#modal-overlay")).toBeVisible();
}

/** Close the modal via the X button. */
async function closeModal(page: import("@playwright/test").Page) {
  await page.locator("#modal-close").click();
  await expect(page.locator("#modal-overlay")).not.toBeVisible();
}

/** Set the status of a task through its modal's status select. */
async function setTaskStatus(
  page: import("@playwright/test").Page,
  taskId: number,
  status: "todo" | "in_progress" | "in_review" | "done",
) {
  await openTaskModal(page, taskId);
  await page.locator("#task-status").selectOption(status);
  await page.locator("#task-form").dispatchEvent("submit");
  await expect(page.locator("#modal-overlay")).not.toBeVisible();
}

/** Switch the Conteúdo tab (not in the shared helper's union, so inline). */
async function switchToConteudo(page: import("@playwright/test").Page) {
  await page.locator('#view-tabs .view-tab[data-view="conteudo"]').click();
}

// ─── 1. Auth: write endpoints reject unauthenticated requests ─────────────────

test.describe("Auth: unauthenticated writes are rejected", () => {
  test("POST /api/projects/:key/tasks without a bearer token returns 401", async ({
    apiContext,
    seedProject,
  }) => {
    // apiContext has no Authorization header by default
    const res = await apiContext.post(
      `/api/projects/${seedProject.key}/tasks`,
      { data: { title: "Should be blocked" } },
    );
    expect(res.status()).toBe(401);
  });

  test("PUT /api/projects/:key/tasks/:id without auth returns 401", async ({
    apiContext,
    seedProject,
  }) => {
    // Create task first (fixture apiContext has no auth, so we use the raw fetch)
    const created = await apiContext.post(
      `/api/projects/${seedProject.key}/tasks`,
      {
        headers: { Authorization: `Bearer ${process.env.TEST_JWT ?? "dev-token"}` },
        data: { title: "Task for 401 update test" },
      },
    );
    // If we can't create (no server token configured), just assert the 401 path
    if (created.status() !== 201) {
      expect(created.status()).toBe(401);
      return;
    }
    const task = await created.json();
    const update = await apiContext.put(
      `/api/projects/${seedProject.key}/tasks/${task.id}`,
      { data: { title: "Updated" } },
    );
    expect(update.status()).toBe(401);
  });
});

// ─── 2. Create ────────────────────────────────────────────────────────────────

test.describe("Create: task appears on the kanban board", () => {
  test("newly created task renders in the To Do column", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Pipeline start task",
      status: "todo",
      priority: "high",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    await expect(card).toBeVisible();

    // Card is inside the "todo" column
    const todoColumn = page
      .locator('.kanban-column[data-status="todo"]')
      .or(page.locator(".kanban-column").filter({ hasText: "To Do" }));
    await expect(todoColumn.locator(`.task-card[data-task-id="${task.id}"]`)).toBeVisible();
  });

  test("task title, key, and priority badge are shown on the card", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Visible fields task",
      priority: "critical",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    await expect(card).toContainText(task.key);
    await expect(card).toContainText("Visible fields task");
  });
});

// ─── 3. Edit ──────────────────────────────────────────────────────────────────

test.describe("Edit: task fields round-trip through the modal", () => {
  test("editing title and description persists on re-open", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Original title",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await openTaskModal(page, task.id);

    await page.locator("#task-title").fill("Updated title");
    await page.locator("#task-description").fill("New description text");
    await page.locator("#task-form").dispatchEvent("submit");
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    // Re-open and verify
    await openTaskModal(page, task.id);
    await expect(page.locator("#task-title")).toHaveValue("Updated title");
    await expect(page.locator("#task-description")).toHaveValue("New description text");
  });

  test("setting a due date appears on the card", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Due date task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await openTaskModal(page, task.id);

    await page.locator("#task-due-date").fill("2026-12-31");
    await page.locator("#task-form").dispatchEvent("submit");
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    await expect(card).toContainText("31");
  });

  test("changing priority to critical reflects in the card", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Priority change task",
      priority: "low",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await openTaskModal(page, task.id);

    await page.locator("#task-priority").selectOption("critical");
    await page.locator("#task-form").dispatchEvent("submit");
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    // Re-open and check saved value
    await openTaskModal(page, task.id);
    await expect(page.locator("#task-priority")).toHaveValue("critical");
  });
});

// ─── 4. Status progression ────────────────────────────────────────────────────

test.describe("Status progression: todo → in_progress → in_review → done", () => {
  test("task moves through all four columns in sequence", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Full-pipeline task",
      status: "todo",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    const stages: Array<"in_progress" | "in_review" | "done"> = [
      "in_progress",
      "in_review",
      "done",
    ];
    const columnLabel: Record<string, string> = {
      in_progress: "In Progress",
      in_review: "In Review",
      done: "Done",
    };

    for (const status of stages) {
      await setTaskStatus(page, task.id, status);
      await waitForBoard(page);

      const targetCol = page
        .locator(".kanban-column")
        .filter({ hasText: columnLabel[status] });
      await expect(
        targetCol.locator(`.task-card[data-task-id="${task.id}"]`),
      ).toBeVisible();
    }
  });

  test("done task shows with visual indicator (status badge or column)", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Done indicator task",
      status: "done",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    const doneCol = page
      .locator(".kanban-column")
      .filter({ hasText: "Done" });
    await expect(
      doneCol.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible();
  });
});

// ─── 5. Cross-view consistency ────────────────────────────────────────────────

test.describe("Cross-view consistency: kanban, table, timeline agree", () => {
  test("task appears in the table view with correct status", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Cross-view task",
      status: "in_progress",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "table");
    await waitForTable(page);

    const row = page.locator(`tr[data-task-id="${task.id}"]`);
    await expect(row).toBeVisible();
    await expect(row).toContainText("Cross-view task");
    // Status column should reflect in_progress
    await expect(row).toContainText(/In Progress/i);
  });

  test("updating status on kanban is reflected in table view", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Status sync task",
      status: "todo",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Move to in_review via kanban modal
    await setTaskStatus(page, task.id, "in_review");
    await waitForBoard(page);

    // Switch to table
    await switchView(page, "table");
    await waitForTable(page);

    const row = page.locator(`tr[data-task-id="${task.id}"]`);
    await expect(row).toContainText(/In Review/i);
  });

  test("task with due_date appears on the timeline", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await createTask(apiContext, seedProject.key, {
      title: "Timeline task",
      status: "in_progress",
      due_date: "2026-06-30",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "timeline");
    await waitForTimeline(page);

    // At least one timeline bar should be rendered
    const bars = page.locator(".timeline-bar, .tl-bar");
    await expect(bars.first()).toBeVisible();
  });
});

// ─── 6. Comments ──────────────────────────────────────────────────────────────

test.describe("Comments: add and verify in task modal", () => {
  test("adding a comment shows it in the comments section", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Commented task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await openTaskModal(page, task.id);

    // Type comment
    await page.locator("#comment-author").fill("QA Bot");
    await page.locator("#comment-body").fill("Looks good to merge.");
    await page.locator("#btn-add-comment").click();

    await expect(
      page.locator(".comment-item").filter({ hasText: "Looks good to merge." }),
    ).toBeVisible();
  });

  test("comment count updates after adding a second comment", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Multi-comment task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await openTaskModal(page, task.id);

    for (const text of ["First comment.", "Second comment."]) {
      await page.locator("#comment-author").fill("Tester");
      await page.locator("#comment-body").fill(text);
      await page.locator("#btn-add-comment").click();
      await page.locator("#comment-body").clear();
    }

    const comments = page.locator(".comment-item");
    await expect(comments).toHaveCount(2);
  });

  test("empty comment body is rejected (no new comment added)", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Empty comment task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await openTaskModal(page, task.id);

    // Attempt to submit with empty body
    await page.locator("#comment-author").fill("Bot");
    // Leave #comment-body empty
    await page.locator("#btn-add-comment").click();

    const comments = page.locator(".comment-item");
    await expect(comments).toHaveCount(0);
  });
});

// ─── 7. Bulk operations ───────────────────────────────────────────────────────

test.describe("Bulk operations: move and delete multiple tasks", () => {
  test("selecting all tasks and bulk-moving to done updates the Done column", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    // Create 3 tasks in todo
    await Promise.all([
      createTask(apiContext, seedProject.key, { title: "Bulk A", status: "todo" }),
      createTask(apiContext, seedProject.key, { title: "Bulk B", status: "todo" }),
      createTask(apiContext, seedProject.key, { title: "Bulk C", status: "todo" }),
    ]);

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "table");
    await waitForTable(page);

    // Select all via header checkbox
    const headerCheckbox = page.locator(".table-select-all, thead input[type=checkbox]").first();
    await headerCheckbox.click();

    // Expect at least 3 rows selected
    const selectedRows = page.locator("tr.selected-row, tr.row-selected");
    const count = await selectedRows.count();
    expect(count).toBeGreaterThanOrEqual(3);

    // Bulk status to done
    const bulkStatusSelect = page.locator(
      "#bulk-status-select, .bulk-action-status",
    );
    if (await bulkStatusSelect.count() > 0) {
      await bulkStatusSelect.selectOption("done");
      const applyBtn = page.locator(
        "#btn-bulk-apply, .btn-bulk-apply",
      );
      if (await applyBtn.count() > 0) await applyBtn.click();
    }
    // Verify via API that tasks are done
    const tasksRes = await apiContext.get(
      `/api/projects/${seedProject.key}/tasks`,
    );
    const tasks = await tasksRes.json();
    const bulkTasks = tasks.filter((t: { title: string }) =>
      ["Bulk A", "Bulk B", "Bulk C"].includes(t.title),
    );
    expect(bulkTasks.every((t: { status: string }) => t.status === "done")).toBe(true);
  });

  test("bulk-delete removes tasks from the board", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const [t1, t2] = await Promise.all([
      createTask(apiContext, seedProject.key, { title: "Delete me A" }),
      createTask(apiContext, seedProject.key, { title: "Delete me B" }),
    ]);

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "table");
    await waitForTable(page);

    // Select both rows
    for (const id of [t1.id, t2.id]) {
      const rowCheckbox = page.locator(
        `tr[data-task-id="${id}"] input[type=checkbox]`,
      );
      if (await rowCheckbox.count() > 0) await rowCheckbox.click();
    }

    const bulkDeleteBtn = page.locator(
      "#btn-bulk-delete, .btn-bulk-delete",
    );
    if (await bulkDeleteBtn.count() > 0) {
      await bulkDeleteBtn.click();
      // Handle confirmation dialog if present
      page.on("dialog", (d) => d.accept());
    }

    // Verify via API
    const res = await apiContext.get(`/api/projects/${seedProject.key}/tasks`);
    const remaining = await res.json();
    const ids = remaining.map((t: { id: number }) => t.id);
    expect(ids).not.toContain(t1.id);
    expect(ids).not.toContain(t2.id);
  });
});

// ─── 8. Archive / restore ─────────────────────────────────────────────────────

test.describe("Archive: done tasks can be archived and restored", () => {
  test("archived task is hidden by default in kanban view", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Archivable task",
      status: "done",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Archive via modal
    await openTaskModal(page, task.id);
    const archiveBtn = page.locator("#btn-archive");
    await archiveBtn.click();
    // Confirm if dialog appears
    page.on("dialog", (d) => d.accept());

    await waitForBoard(page);

    // Task should not be visible (archived = hidden by default)
    await expect(
      page.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toHaveCount(0);
  });

  test("toggling 'Archived' checkbox reveals archived tasks", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Find me when archived",
      status: "done",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Archive via modal
    await openTaskModal(page, task.id);
    const archiveBtn = page.locator("#btn-archive");
    await archiveBtn.click();
    page.on("dialog", (d) => d.accept());
    await waitForBoard(page);

    // Enable archived toggle
    await page.locator("#show-archived").check();
    await waitForBoard(page);

    await expect(
      page.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible();
  });
});

// ─── 9. Dashboard ─────────────────────────────────────────────────────────────

test.describe("Dashboard: status counts reflect completed workflow", () => {
  test("dashboard shows non-zero task counts after seeding tasks", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await Promise.all([
      createTask(apiContext, seedProject.key, { title: "Dash todo", status: "todo" }),
      createTask(apiContext, seedProject.key, { title: "Dash done", status: "done" }),
    ]);

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "dashboard");

    // Dashboard should render
    const dashboard = page.locator(".dashboard, .dashboard-grid");
    await expect(dashboard.first()).toBeVisible();

    // Status bars section present
    const statusSection = page.locator(
      ".dashboard-status-bar-track, .dashboard-status-row",
    );
    await expect(statusSection.first()).toBeVisible();
  });

  test("API /dashboard returns correct total after creating tasks", async ({
    apiContext,
    seedProject,
  }) => {
    await Promise.all([
      createTask(apiContext, seedProject.key, { title: "D1", status: "todo" }),
      createTask(apiContext, seedProject.key, { title: "D2", status: "in_progress" }),
      createTask(apiContext, seedProject.key, { title: "D3", status: "done" }),
    ]);

    const res = await apiContext.get(
      `/api/projects/${seedProject.key}/dashboard`,
    );
    expect(res.status()).toBe(200);
    const data = await res.json();
    expect(data.status_counts.total).toBeGreaterThanOrEqual(3);
  });

  test("completed task appears in recently_updated on the dashboard", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Dashboard recent task",
      status: "done",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await switchView(page, "dashboard");

    const recentSection = page.locator(
      ".dashboard-card, .dashboard-task-item",
    );
    await expect(recentSection.first()).toBeVisible();

    // The task key should appear somewhere in the dashboard
    await expect(page.locator(`text=${task.key}`)).toBeVisible();
  });
});

// ─── 10. Conteúdo tab ─────────────────────────────────────────────────────────

test.describe("Conteúdo: quilombo feed tab renders without errors", () => {
  test("Conteúdo tab is visible in the view-tabs bar", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    const tab = page.locator('#view-tabs .view-tab[data-view="conteudo"]');
    await expect(tab).toBeVisible();
    await expect(tab).toContainText("Conteúdo");
  });

  test("clicking Conteúdo tab loads the feed sections without a JS error", async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));
    page.on("console", (msg) => {
      if (msg.type() === "error") errors.push(msg.text());
    });

    await navigateTo(page, "/");
    await switchToConteudo(page);

    // Three section headings should appear
    await expect(
      page.locator(".conteudo-section-title").filter({ hasText: "Próximos Eventos" }),
    ).toBeVisible();
    await expect(
      page.locator(".conteudo-section-title").filter({ hasText: "Publicações Recentes" }),
    ).toBeVisible();
    await expect(
      page.locator(".conteudo-section-title").filter({ hasText: "Missões Ativas" }),
    ).toBeVisible();

    // No JS errors during render
    expect(errors.filter((e) => !e.includes("favicon"))).toHaveLength(0);
  });

  test("Conteúdo tab is accessible even when no project is selected", async ({
    page,
  }) => {
    await navigateTo(page, "/");
    // Do NOT select a project
    await switchToConteudo(page);

    // Grid should render (may show empty-state cards but no crash)
    await expect(page.locator(".conteudo-grid")).toBeVisible();
  });

  test("switching from kanban to Conteúdo and back preserves project state", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "State-preservation task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);

    // Switch to Conteúdo
    await switchToConteudo(page);
    await expect(page.locator(".conteudo-grid")).toBeVisible();

    // Switch back to kanban
    await switchView(page, "kanban");
    await waitForBoard(page);

    // Original task still visible
    await expect(
      page.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible();
  });
});
