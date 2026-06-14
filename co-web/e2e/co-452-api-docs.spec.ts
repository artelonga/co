/**
 * CO-452 — Public API docs (Swagger UI + OpenAPI discovery).
 *
 * Verifies the discovery endpoint and that the vendored Swagger UI renders the
 * documented endpoint list (no external CDN involved).
 */

import { test, expect } from "@playwright/test";

test.describe("CO-452 — API docs", () => {
  test("GET /api/openapi.json → 200, openapi 3.x, X-API-Version: v1", async ({
    request,
  }) => {
    const res = await request.get("/api/openapi.json");
    expect(res.status()).toBe(200);
    expect(res.headers()["content-type"]).toContain("application/json");
    expect(res.headers()["x-api-version"]).toBe("v1");

    const body = await res.json();
    expect(String(body.openapi)).toMatch(/^3\./);
    expect(body.paths).toBeTruthy();
    expect(Object.keys(body.paths).length).toBeGreaterThan(0);
  });

  test("GET /api/docs → 200, Swagger UI loads the endpoint list", async ({
    page,
  }) => {
    const res = await page.goto("/api/docs");
    expect(res?.status()).toBe(200);

    // The vendored bundle (same-origin) boots Swagger UI, which fetches
    // /api/openapi.json and renders one <div> per documented operation.
    await expect(page.locator(".swagger-ui")).toBeVisible();
    await expect(page.locator(".opblock").first()).toBeVisible({
      timeout: 15000,
    });
  });

  test("Swagger UI assets are served same-origin, not from a CDN", async ({
    page,
  }) => {
    await page.goto("/api/docs");
    const cssHref = await page
      .locator('link[rel="stylesheet"]')
      .first()
      .getAttribute("href");
    expect(cssHref).toContain("/shared/swagger/");
    expect(cssHref).not.toContain("unpkg.com");

    const cssRes = await page.request.get("/shared/swagger/swagger-ui.css");
    expect(cssRes.status()).toBe(200);
    const jsRes = await page.request.get(
      "/shared/swagger/swagger-ui-bundle.js",
    );
    expect(jsRes.status()).toBe(200);
  });
});
