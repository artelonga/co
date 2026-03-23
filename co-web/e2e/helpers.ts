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
  },
): Promise<{ id: number; key: string; title: string; status: string }> {
  const res = await apiContext.post(`/api/projects/${projectKey}/tasks`, {
    data: {
      title: taskData.title,
      description: taskData.description ?? "",
      status: taskData.status ?? "todo",
      priority: taskData.priority ?? "medium",
      labels: taskData.labels ?? [],
    },
  });
  expect(res.status()).toBe(201);
  return res.json();
}

/** Count visible task cards on the current page */
export async function getTaskCount(page: Page): Promise<number> {
  return page.locator(".task-card").count();
}
