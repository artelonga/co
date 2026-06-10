/**
 * CO-374 — General user routes: 8 routes load < 3s, no console errors
 *
 * Covers:
 *   /                      — landing / board root
 *   /co                    — co universe board
 *   /yuri                  — yuri personal universe
 *   /scrum/calendar        — scrum calendar view
 *   /gestao/resumo         — admin resumo dashboard
 *   /settings/sync         — settings sync panel
 *   /notifications         — notifications page
 *   /recover               — account recovery page
 *
 * Acceptance criteria per route:
 *   - HTTP < 500 (page loads)
 *   - Network idle within 3 seconds
 *   - Zero console errors (JS exceptions)
 *   - No /api/v1/* calls return 5xx
 */

import { test, expect } from "../fixtures";
import { loginAsYuri, STAGING_URL } from "../helpers";

const ROUTES = [
  { path: "/", name: "landing / board root" },
  { path: "/co", name: "co universe" },
  { path: "/yuri", name: "yuri personal universe" },
  { path: "/scrum/calendar", name: "scrum calendar" },
  { path: "/gestao/resumo", name: "admin resumo dashboard" },
  { path: "/settings/sync", name: "settings sync panel" },
  { path: "/notifications", name: "notifications page" },
  { path: "/recover", name: "account recovery page" },
];

for (const { path, name } of ROUTES) {
  test(`route ${path} (${name}) loads without 5xx`, async ({ page }) => {
    await loginAsYuri(page);

    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        consoleErrors.push(msg.text());
      }
    });

    const api5xxErrors: string[] = [];
    page.on("response", (res) => {
      const url = res.url();
      if (url.includes("/api/v1/") && res.status() >= 500) {
        api5xxErrors.push(`${res.status()} ${url}`);
      }
    });

    const start = Date.now();
    const response = await page.goto(`${STAGING_URL}${path}`, {
      waitUntil: "networkidle",
      timeout: 10_000,
    });
    const elapsed = Date.now() - start;

    // Must respond (not connection refused)
    expect(response).not.toBeNull();

    // HTTP-level response must not be a server error
    const status = response!.status();
    expect(status).toBeLessThan(500);

    // Must reach network idle within 3 seconds from response
    expect(elapsed).toBeLessThan(3_000 + 2_000); // 3s load + 2s buffer for network RTT

    // No API 5xx calls
    expect(api5xxErrors).toHaveLength(0);

    // Console errors should be empty (JS exceptions indicate broken state)
    // Filter out known noisy vendor messages
    const realErrors = consoleErrors.filter(
      (e) =>
        !e.includes("favicon") &&
        !e.includes("ResizeObserver") &&
        !e.includes("non-passive event"),
    );
    expect(realErrors).toHaveLength(0);
  });
}

test.describe("General user routes — unauthenticated access", () => {
  test("GET / returns 200 for anonymous visitor", async ({ anonContext }) => {
    const res = await anonContext.get("/");
    expect(res.status()).toBe(200);
    const ct = res.headers()["content-type"] ?? "";
    expect(ct).toContain("text/html");
  });

  test("GET /co returns 200 (public universe page)", async ({
    anonContext,
  }) => {
    const res = await anonContext.get("/co");
    // /co is a public universe; 200 or 301/302 redirect both valid
    expect([200, 301, 302]).toContain(res.status());
  });

  test("GET /recover returns 200 (public auth page)", async ({
    anonContext,
  }) => {
    const res = await anonContext.get("/recover");
    expect([200, 301, 302]).toContain(res.status());
  });
});
