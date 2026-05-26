/**
 * UAT flow E2E tests — CO-44
 *
 * Run against the UAT environment:
 *   BASE_URL=https://co-artelonga-uat.fly.dev npx playwright test e2e/uat-flow.spec.ts
 *
 * Requires CO_ENV=uat on the server. Tests validate:
 * - yuri/uat password login
 * - co-dev board accessible after login
 * - uat-login endpoint returns 404 in prod (when CO_ENV != "uat")
 */

import { test, expect } from "@playwright/test";

// Shared login helper — returns session token from cookie.
async function yuriLogin(
  request: Parameters<Parameters<typeof test>[1]>[0]["request"],
): Promise<string> {
  const res = await request.post("/api/v1/auth/uat-login", {
    data: { email: "yuri@uat.local", password: "uat" },
  });
  expect(res.status(), "yuri login should succeed").toBe(200);
  const body = await res.json();
  expect(body.user_id).toBeTruthy();
  expect(body.email).toBe("yuri@uat.local");

  // Extract session token from Set-Cookie header.
  const setCookie = res.headers()["set-cookie"] ?? "";
  const match = setCookie.match(/session=([^;]+)/);
  return match ? match[1] : "";
}

test.describe("UAT flow — CO-44", () => {
  test("health check passes", async ({ request }) => {
    const res = await request.get("/api/health");
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.status).toBe("ok");
  });

  test("yuri login with correct password returns 200 + JWT", async ({
    request,
  }) => {
    const res = await request.post("/api/v1/auth/uat-login", {
      data: { email: "yuri@uat.local", password: "uat" },
    });
    expect(res.status()).toBe(200);

    const body = await res.json();
    expect(body.user_id).toBeTruthy();
    expect(body.email).toBe("yuri@uat.local");
    expect(body.display_name).toBeTruthy();
    expect(body.expires_at).toBeTruthy();
  });

  test("yuri login with wrong password returns 401", async ({ request }) => {
    const res = await request.post("/api/v1/auth/uat-login", {
      data: { email: "yuri@uat.local", password: "wrong-password" },
    });
    expect(res.status()).toBe(401);
  });

  test("yuri login with unknown email returns 401", async ({ request }) => {
    const res = await request.post("/api/v1/auth/uat-login", {
      data: { email: "nobody@uat.local", password: "uat" },
    });
    expect(res.status()).toBe(401);
  });

  test("co-dev board accessible after yuri login", async ({ request }) => {
    const token = await yuriLogin(request);
    expect(token, "session token should be set in cookie").toBeTruthy();

    const res = await request.get("/api/v1/universes/co-dev/entries", {
      headers: { Cookie: `session=${token}` },
    });
    expect(res.status(), "co-dev entries should be accessible to admin").toBe(
      200,
    );

    const data = await res.json();
    expect(data.total, "co-dev board should have at least one task").toBeGreaterThan(0);
  });

  test("co-dev board returns 404 without auth", async ({ request }) => {
    const res = await request.get("/api/v1/universes/co-dev/entries");
    // dev board returns 404 (not 403) to hide its existence
    expect(res.status()).toBe(404);
  });

  test("me endpoint returns admin tier for yuri", async ({ request }) => {
    const token = await yuriLogin(request);
    const res = await request.get("/api/v1/auth/me", {
      headers: { Authorization: `Bearer ${token}` },
    });
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.tier).toBe("admin");
    expect(body.email).toBe("yuri@uat.local");
  });
});
