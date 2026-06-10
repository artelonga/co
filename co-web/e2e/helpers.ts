import { type Page, type APIRequestContext, expect } from "@playwright/test";

/** Navigate to a path and wait for network to settle */
export async function navigateTo(page: Page, path: string): Promise<void> {
  await page.goto(path, { waitUntil: "networkidle" });
}

/**
 * Wait for the kanban board to fully render.
 *
 * STATUSES in constants.js defines 3 columns (todo / in_progress / done).
 * Uses a single waitForSelector on .kanban-column to avoid the two-step
 * race where .kanban appears but columns are not yet painted.
 */
export async function waitForBoard(page: Page): Promise<void> {
  await page.waitForSelector(".kanban-column", { state: "visible", timeout: 10_000 });
}

/**
 * Wait for the board to be fully ready for interaction.
 *
 * Confirms the kanban DOM is rendered. Call after `selectProject` which
 * already ensures kanban columns are visible before returning.
 */
export async function waitForBoardReady(page: Page): Promise<void> {
  await page.waitForSelector(".kanban-column", { state: "visible", timeout: 10_000 });
}

/** Wait for the table view to render */
export async function waitForTable(page: Page): Promise<void> {
  await page.waitForSelector(".table-container", { state: "visible" });
}

/** Wait for the timeline view to render */
export async function waitForTimeline(page: Page): Promise<void> {
  await page.waitForSelector(".timeline-wrapper", { state: "visible" });
}

/**
 * Open the sidebar drawer on narrow mobile viewports (≤ 640 px).
 * No-op on wider viewports or when the sidebar is already open.
 * CO-359: required after CO-358's drawer sidebar landed for mobile.
 */
export async function openSidebarIfMobile(page: Page): Promise<void> {
  const vp = page.viewportSize();
  if (!vp || vp.width > 640) return;
  const hamburger = page.locator("#hamburger-btn");
  if ((await hamburger.count()) === 0) return;
  const sidebar = page.locator("#sidebar");
  const isOpen = await sidebar
    .evaluate((el) => el.classList.contains("open"))
    .catch(() => false);
  if (!isOpen) {
    await hamburger.click();
    await page.waitForFunction(
      () => document.getElementById("sidebar")?.classList.contains("open"),
      { timeout: 3_000 },
    );
  }
}

/** Click a project in the sidebar and wait for the kanban board */
export async function selectProject(page: Page, key: string): Promise<void> {
  await openSidebarIfMobile(page);
  const link = page.locator(
    `#project-list .sidebar-item-key:text-is("${key}")`,
  );
  // Register the response listener BEFORE clicking so we don't miss the request.
  // waitForResponse resolves after the HTTP body arrives, by which point the JS
  // microtask chain (hideLoading() + render()) has already run — state.loading is
  // guaranteed false before we switch to kanban.
  const tasksLoaded = page.waitForResponse(
    (r) =>
      r.url().includes(`/api/projects/${key}/tasks`) && r.status() === 200,
    { timeout: 15_000 },
  );
  await link.click();
  await tasksLoaded;
  // CO-359: on mobile the drawer stays open after selection and intercepts
  // every later interaction — close it. The open sidebar covers the
  // overlay's center, so dispatch the click straight to the overlay.
  const vp = page.viewportSize();
  if (vp && vp.width <= 640) {
    const overlay = page.locator("#sidebar-overlay.visible");
    if ((await overlay.count()) > 0) {
      await overlay.dispatchEvent("click");
      await page.waitForFunction(
        () => !document.getElementById("sidebar")?.classList.contains("open"),
        { timeout: 3_000 },
      );
    }
  }
  // CO-304: render() fires renderConteudo() which is async (6 API calls). Those
  // calls complete and overwrite content.innerHTML AFTER we switch to kanban.
  // renderConteudo() now has a stale-view guard, but we also wait here for the
  // loading spinner to be gone so the conteudo API calls have settled first.
  await page.waitForFunction(
    () => !document.querySelector('#content .loading-spinner'),
    { timeout: 10_000 }
  ).catch(() => {});
  // Default view is conteudo (CO-304). Switch to kanban for board interaction tests.
  await page.locator('#view-tabs .view-tab[data-view="kanban"]').click();
  await page.waitForSelector(".kanban-column", { state: "visible", timeout: 10_000 });
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

/**
 * Login as admin. On UAT uses uat-login (magic credentials, no env vars needed);
 * on prod / local uses password-login via CO_ADMIN_EMAIL + CO_ADMIN_PASSWORD env vars.
 *
 * The POST sets a session cookie on the page context automatically — subsequent
 * page.goto() calls will carry the cookie without any extra setup.
 */
export async function loginAsAdmin(page: Page): Promise<void> {
  const base = process.env.BASE_URL ?? "http://localhost:3000";
  if (base.includes("uat")) {
    // UAT: magic credentials, no env vars needed
    await page.request.post(`${base}/api/v1/auth/uat-login`, {
      data: { email: "yuri@uat.local", password: "uat" },
    });
  } else {
    // Prod / local: CO_ADMIN_EMAIL + CO_ADMIN_PASSWORD env vars
    const email = process.env.CO_ADMIN_EMAIL ?? "";
    const password = process.env.CO_ADMIN_PASSWORD ?? "";
    await page.request.post(`${base}/api/v1/auth/password-login`, {
      data: { email, password },
    });
  }
}
