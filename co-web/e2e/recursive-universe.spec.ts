/**
 * CO-264 — Universe = recursive folder tree
 *
 * Asserts:
 * - `GET /api/v1/universes/co/entries?path_prefix=public/` returns only `public/*` entries.
 * - `GET /co/changelog` returns HTTP 200 (CHANGELOG.md is seeded in the `co` universe).
 * - `GET /artelonga/changelog` returns 200 or 404 (graceful — seeded when cross-repo sync lands).
 * - `GET /co/public/` returns HTTP 200 (folder-level trailing-slash route).
 * - `path_prefix` filter works for any universe, not just `co`.
 */

import { test, expect, request as playwrightRequest } from "@playwright/test";

test.describe("CO-264: path_prefix entry filter", () => {
  test("GET /api/v1/universes/co/entries?path_prefix=public/ returns only public/* entries", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });

    const res = await ctx.get(
      "/api/v1/universes/co/entries?path_prefix=public/",
    );
    expect(res.status()).toBe(200);

    const body = await res.json();
    expect(body).toHaveProperty("entries");
    expect(body).toHaveProperty("total");
    expect(Array.isArray(body.entries)).toBe(true);

    for (const entry of body.entries) {
      expect(
        entry.path.startsWith("public/"),
        `expected path "${entry.path}" to start with "public/"`,
      ).toBe(true);
    }

    await ctx.dispose();
  });

  test("path_prefix=public/ total matches entries length", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });

    const res = await ctx.get(
      "/api/v1/universes/co/entries?path_prefix=public/",
    );
    const body = await res.json();
    expect(body.total).toBe(body.entries.length);

    await ctx.dispose();
  });

  test("path_prefix= with empty prefix returns all entries (no filter)", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });

    const all = await ctx.get("/api/v1/universes/template/entries");
    const prefixed = await ctx.get(
      "/api/v1/universes/template/entries?path_prefix=",
    );

    expect(all.status()).toBe(200);
    expect(prefixed.status()).toBe(200);

    // Both should return the same set of entries when prefix is empty string.
    const allBody = await all.json();
    const prefixedBody = await prefixed.json();
    expect(prefixedBody.total).toBe(allBody.total);

    await ctx.dispose();
  });
});

test.describe("CO-264: universe-scoped changelog route", () => {
  test("GET /co/changelog returns HTTP 200", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });

    const res = await ctx.get("/co/changelog");
    expect(res.status()).toBe(200);

    const ct = res.headers()["content-type"] ?? "";
    expect(ct).toContain("text/html");

    await ctx.dispose();
  });

  test("GET /artelonga/changelog returns 200 or 404 (graceful)", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });

    const res = await ctx.get("/artelonga/changelog");
    // 200 when CHANGELOG.md is seeded; 404 when cross-repo sync hasn't run yet.
    expect([200, 404]).toContain(res.status());

    await ctx.dispose();
  });

  test("/co/changelog SPA renders changelog entry or empty state", async ({
    page,
  }) => {
    await page.goto("/co/changelog", { waitUntil: "domcontentloaded" });

    // Either the zoom modal opens (CHANGELOG.md found) or the not-found view appears.
    const zoomModal = page.locator("#zoom-modal, .zoom-modal, [id*=zoom]");
    const notFoundView = page.locator("#co-not-found-view");

    await expect(zoomModal.or(notFoundView)).toBeVisible({ timeout: 12_000 });
  });
});

test.describe("CO-264: folder-level trailing-slash routing", () => {
  test("GET /co/public/ returns HTTP 200", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });

    const res = await ctx.get("/co/public/");
    expect(res.status()).toBe(200);

    const ct = res.headers()["content-type"] ?? "";
    expect(ct).toContain("text/html");

    await ctx.dispose();
  });

  test("GET /template/ returns HTTP 200 (universe root trailing slash)", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });

    const res = await ctx.get("/template/");
    expect(res.status()).toBe(200);

    await ctx.dispose();
  });
});
