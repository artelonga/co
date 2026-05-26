/**
 * CO-264 — Recursive universe / path_prefix / changelog route.
 * Thinned (CO-302): core API shape + route responses only.
 */

import { test, expect, request as playwrightRequest } from "@playwright/test";

test.describe("CO-264: path_prefix filter", () => {
  test("GET /api/v1/universes/co/entries?path_prefix=public/ returns only public/* entries", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });
    const res = await ctx.get("/api/v1/universes/co/entries?path_prefix=public/");
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body).toHaveProperty("entries");
    expect(Array.isArray(body.entries)).toBe(true);
    for (const entry of body.entries) {
      expect(entry.path.startsWith("public/")).toBe(true);
    }
    await ctx.dispose();
  });

  test("path_prefix total matches entries length", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });
    const res = await ctx.get("/api/v1/universes/co/entries?path_prefix=public/");
    const body = await res.json();
    expect(body.total).toBe(body.entries.length);
    await ctx.dispose();
  });
});

test.describe("CO-264: changelog route", () => {
  test("GET /co/changelog returns HTTP 200 HTML", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });
    const res = await ctx.get("/co/changelog");
    expect(res.status()).toBe(200);
    expect((res.headers()["content-type"] ?? "")).toContain("text/html");
    await ctx.dispose();
  });
});

test.describe("CO-264: trailing-slash routing", () => {
  test("GET /co/public/ returns HTTP 200", async () => {
    const base = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwrightRequest.newContext({ baseURL: base });
    const res = await ctx.get("/co/public/");
    expect(res.status()).toBe(200);
    await ctx.dispose();
  });
});
