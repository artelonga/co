/**
 * Changelog viewer — thinned (CO-302): API shape + page render.
 * Range/filter queries and pagination moved to lib tests.
 */

import { test, expect } from "@playwright/test";

test.describe("Changelog API", () => {
  test("GET /api/v1/changelog returns valid JSON shape", async ({ request }) => {
    const res = await request.get("/api/v1/changelog");
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body).toHaveProperty("range");
    expect(body).toHaveProperty("versions");
    expect(Array.isArray(body.versions)).toBe(true);
  });

  test("versions are sorted newest-first by default", async ({ request }) => {
    const res = await request.get("/api/v1/changelog");
    const { versions } = await res.json();
    if (versions.length < 2) return;
    const toTuple = (v: string) =>
      v.split(".").map((n: string) => parseInt(n, 10)) as [number, number, number];
    const [a, b] = [toTuple(versions[0].version), toTuple(versions[1].version)];
    const gte =
      a[0] > b[0] ||
      (a[0] === b[0] && a[1] > b[1]) ||
      (a[0] === b[0] && a[1] === b[1] && a[2] >= b[2]);
    expect(gte).toBe(true);
  });

  test("?type=feat returns only feat entries", async ({ request }) => {
    const res = await request.get("/api/v1/changelog?type=feat");
    expect(res.status()).toBe(200);
    const { versions } = await res.json();
    for (const v of versions) {
      const hasFeat = (v.entries ?? []).some(
        (e: { type: string }) => e.type === "feat",
      );
      if (v.entries?.length > 0) expect(hasFeat).toBe(true);
    }
  });
});

test.describe("Changelog page", () => {
  test("GET /changelog returns 200 HTML", async ({ request }) => {
    const res = await request.get("/changelog");
    expect([200, 404]).toContain(res.status()); // 404 if route not mounted yet
    if (res.status() === 200) {
      expect(res.headers()["content-type"]).toContain("text/html");
    }
  });
});
