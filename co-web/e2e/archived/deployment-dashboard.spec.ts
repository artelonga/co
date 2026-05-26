/**
 * CO-273: Deployment dashboard E2E spec.
 *
 * The dashboard is admin-only (requires CO_SEED_ADMIN_EMAIL match).
 * These tests verify the endpoint exists and is properly protected.
 * Full rendering tests require admin credentials (run manually or via UAT).
 */

import { test, expect } from "./fixtures";

test.describe("CO-273: Deployment dashboard", () => {
  test("GET /api/v1/admin/deployments requires auth (returns 401)", async ({
    apiContext,
  }) => {
    const res = await apiContext.get("/api/v1/admin/deployments");
    // 401 = endpoint exists and is protected; 404 would mean it's missing.
    expect(res.status()).toBe(401);
  });

  test("POST /api/v1/admin/deployments/refresh requires auth (returns 401)", async ({
    apiContext,
  }) => {
    const res = await apiContext.post("/api/v1/admin/deployments/refresh");
    expect(res.status()).toBe(401);
  });

  test("/admin/deployments redirects unauthenticated users", async ({
    page,
  }) => {
    // The page redirects to / when no valid admin session cookie is present.
    await page.goto("/admin/deployments", { waitUntil: "commit" });
    expect(page.url()).not.toContain("/admin/deployments");
  });
});
