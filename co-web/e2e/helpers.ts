import { type Page, type APIRequestContext, expect } from "@playwright/test";

/** Navigate to a path and wait for network to settle */
export async function navigateTo(page: Page, path: string): Promise<void> {
  await page.goto(path, { waitUntil: "networkidle" });
}

/** Wait for the kanban board to fully render (all 4 status columns visible) */
export async function waitForBoard(page: Page): Promise<void> {
  await page.waitForSelector(".kanban", { state: "visible" });
  const columns = page.locator(".kanban-column");
  await expect(columns).toHaveCount(4);
}

/** Wait for the table view to render */
export async function waitForTable(page: Page): Promise<void> {
  await page.waitForSelector(".table-container", { state: "visible" });
}

/** Wait for the timeline view to render */
export async function waitForTimeline(page: Page): Promise<void> {
  await page.waitForSelector(".timeline-wrapper", { state: "visible" });
}

/** Click a project in the sidebar and wait for board */
export async function selectProject(page: Page, key: string): Promise<void> {
  const link = page.locator(
    `#project-list .sidebar-item-key:text-is("${key}")`,
  );
  await link.click();
  await waitForBoard(page);
}

/** Click a view tab by name */
export async function switchView(
  page: Page,
  view: "kanban" | "table" | "timeline" | "calendar" | "dashboard",
): Promise<void> {
  await page.locator(`#view-tabs .view-tab[data-view="${view}"]`).click();
}

/** Create a task via the API and return the created task object */
export async function createTask(
  apiContext: APIRequestContext,
  projectKey: string,
  taskData: {
    title: string;
    description?: string;
    status?: string;
    priority?: string;
    labels?: string[];
    parent?: number;
    due_date?: string;
  },
): Promise<{ id: number; key: string; title: string; status: string }> {
  const body: Record<string, unknown> = {
    title: taskData.title,
    description: taskData.description ?? "",
    status: taskData.status ?? "todo",
    priority: taskData.priority ?? "medium",
    labels: taskData.labels ?? [],
  };
  if (taskData.parent !== undefined) body.parent = taskData.parent;
  if (taskData.due_date !== undefined) body.due_date = taskData.due_date;

  const res = await apiContext.post(`/api/projects/${projectKey}/tasks`, {
    data: body,
  });
  expect(res.status()).toBe(201);
  return res.json();
}

/** Count visible task cards on the current page */
export async function getTaskCount(page: Page): Promise<number> {
  return page.locator(".task-card").count();
}
