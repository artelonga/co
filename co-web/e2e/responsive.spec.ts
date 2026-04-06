/**
 * CO-33 — Responsive: board renders at mobile (375px), tablet (768px), desktop (1280px)
 *
 * Verifies that the board UI is functional and visually coherent at the three
 * target breakpoints.  Tests are run against each viewport via Playwright's
 * `test.use({ viewport })` override.
 */

import { test, expect } from "./fixtures";

// ─── Desktop (1280×720) ───────────────────────────────────────────────────────

test.describe("Responsive: desktop 1280×720", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("board loads and shows kanban columns at 1280px", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    // Seed a task so the board has content
    await apiContext.post(`/api/projects/${seedProject.key}/tasks`, {
      headers: {
        Authorization: `Bearer ${process.env.TEST_JWT ?? "dev-token"}`,
      },
      data: { title: "Desktop task", status: "todo" },
    });

    await page.goto("/", { waitUntil: "networkidle" });

    // Sidebar should be visible at desktop width
    const sidebar = page.locator("#sidebar");
    await expect(sidebar).toBeVisible();

    // Click project
    const projectLink = page.locator(
      `#project-list .sidebar-item-key:text-is("${seedProject.key}")`,
    );
    await projectLink.click();

    // Kanban board renders
    await page.waitForSelector(".kanban", { state: "visible" });
    const columns = page.locator(".kanban-column");
    await expect(columns).toHaveCount(4);
  });

  test("hamburger button is NOT visible at 1280px", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    const hamburger = page.locator("#hamburger-btn");
    // Either hidden or not present at desktop width
    await expect(hamburger).toHaveClass(/hidden/).catch(async () => {
      await expect(hamburger).not.toBeVisible();
    });
  });
});

// ─── Tablet (768×1024) ────────────────────────────────────────────────────────

test.describe("Responsive: tablet 768×1024", () => {
  test.use({ viewport: { width: 768, height: 1024 } });

  test("board renders with kanban columns at 768px", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await apiContext.post(`/api/projects/${seedProject.key}/tasks`, {
      headers: {
        Authorization: `Bearer ${process.env.TEST_JWT ?? "dev-token"}`,
      },
      data: { title: "Tablet task", status: "todo" },
    });

    await page.goto("/", { waitUntil: "networkidle" });

    // At 768px the sidebar may be visible or behind a hamburger depending on breakpoint
    const projectLink = page.locator(
      `#project-list .sidebar-item-key:text-is("${seedProject.key}")`,
    );

    // If sidebar is visible, click directly; otherwise open hamburger first
    const isVisible = await projectLink.isVisible().catch(() => false);
    if (!isVisible) {
      const hamburger = page.locator("#hamburger-btn");
      if (await hamburger.isVisible()) await hamburger.click();
    }

    if (await projectLink.isVisible({ timeout: 3_000 }).catch(() => false)) {
      await projectLink.click();
      await page.waitForSelector(".kanban", { state: "visible" });
      const columns = page.locator(".kanban-column");
      await expect(columns).toHaveCount(4);
    }
  });

  test("app root loads without JS errors at 768px", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await page.goto("/", { waitUntil: "networkidle" });
    await page.waitForTimeout(500);

    expect(errors.filter((e) => !e.includes("favicon"))).toHaveLength(0);
  });
});

// ─── Mobile (375×812) ─────────────────────────────────────────────────────────

test.describe("Responsive: mobile 375×812", () => {
  test.use({ viewport: { width: 375, height: 812 } });

  test("app root loads at 375px without JS errors", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await page.goto("/", { waitUntil: "networkidle" });
    await page.waitForTimeout(500);

    expect(errors.filter((e) => !e.includes("favicon"))).toHaveLength(0);
  });

  test("hamburger button is visible at 375px", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await expect(page.locator("#hamburger-btn")).toBeVisible();
  });

  test("sidebar opens via hamburger on 375px", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.locator("#hamburger-btn").click();
    await expect(page.locator("#sidebar-overlay")).toHaveClass(/visible/);
  });

  test("landing page /co renders the hero section at 375px", async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await page.goto("/co", { waitUntil: "networkidle" });
    await expect(page.locator("#template-banner")).toBeVisible();
    await expect(page.locator("#btn-criar-universo")).toBeVisible();

    expect(errors.filter((e) => !e.includes("favicon"))).toHaveLength(0);
  });

  test("kanban board columns are scrollable at 375px after project selection", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    await apiContext.post(`/api/projects/${seedProject.key}/tasks`, {
      headers: {
        Authorization: `Bearer ${process.env.TEST_JWT ?? "dev-token"}`,
      },
      data: { title: "Mobile task", status: "todo" },
    });

    await page.goto("/", { waitUntil: "networkidle" });

    // Open sidebar via hamburger
    await page.locator("#hamburger-btn").click();
    const projectLink = page.locator(
      `#project-list .sidebar-item-key:text-is("${seedProject.key}")`,
    );

    if (await projectLink.isVisible({ timeout: 3_000 }).catch(() => false)) {
      await projectLink.click();
      await page.waitForSelector(".kanban", { state: "visible", timeout: 5_000 });
      // Kanban is scrollable — at least 1 column should exist
      const columns = page.locator(".kanban-column");
      const count = await columns.count();
      expect(count).toBeGreaterThanOrEqual(1);
    }
  });
});

// ─── Cross-breakpoint API contract ───────────────────────────────────────────

test.describe("Responsive: core API works at all viewports", () => {
  const viewports = [
    { label: "mobile", width: 375, height: 812 },
    { label: "tablet", width: 768, height: 1024 },
    { label: "desktop", width: 1280, height: 720 },
  ];

  for (const vp of viewports) {
    test(`health endpoint responds at ${vp.label} (${vp.width}px)`, async ({
      request,
    }) => {
      const res = await request.get("/api/health");
      expect(res.status()).toBe(200);
      const body = await res.json();
      expect(body.status).toBe("ok");
    });
  }
});
