import { test, expect } from "./fixtures";
import { navigateTo, waitForBoard, createTask, getTaskCount } from "./helpers";

test.describe("Smoke tests", () => {
  test("server is running and healthy", async ({ apiContext }) => {
    const res = await apiContext.get("/api/health");
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.status).toBe("ok");
    expect(body.version).toBeTruthy();
  });

  test("can list projects via API", async ({ apiContext }) => {
    const res = await apiContext.get("/api/projects");
    expect(res.status()).toBe(200);
    const projects = await res.json();
    expect(Array.isArray(projects)).toBe(true);
  });

  test("board loads for a seeded project", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    // Skip on mobile viewports — sidebar uses display:none at <=640px
    const viewport = page.viewportSize();
    test.skip(
      !!(viewport && viewport.width <= 640),
      "Sidebar not available on mobile viewport",
    );

    // Create a task so the board isn't empty
    await createTask(apiContext, seedProject.key, { title: "Smoke task" });

    // Navigate to the app root
    await navigateTo(page, "/");

    // Click on the seeded project in the sidebar
    const projectLink = page.locator(
      `#project-list .sidebar-item-key:text-is("${seedProject.key}")`,
    );
    await projectLink.click();

    // Wait for the kanban board to appear
    await waitForBoard(page);

    // Should have at least 1 task card
    const count = await getTaskCount(page);
    expect(count).toBeGreaterThanOrEqual(1);
  });
});
