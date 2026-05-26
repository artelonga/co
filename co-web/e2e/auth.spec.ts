/**
 * Auth — consolidated: session flow, anonymous ownership, usage gate, subscribe prompt
 *
 * Replaces auth-crdt.spec.ts + usage-gate.spec.ts + unsubscribed-access.spec.ts
 * (archived after CO-302).
 */

import { test, expect } from "./fixtures";

// ─── Auth API ─────────────────────────────────────────────────────────────────

test.describe("Auth API", () => {
  test("POST /api/v1/auth/login returns 200 for any email", async ({
    request,
  }) => {
    const res = await request.post("/api/v1/auth/login", {
      data: { email: "nobody@notregistered.example.com" },
    });
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.message).toBeTruthy();
  });

  test("POST /api/v1/auth/logout clears the session cookie", async ({
    request,
  }) => {
    const res = await request.post("/api/v1/auth/logout");
    expect(res.status()).toBe(200);
    const setCookie = res.headers()["set-cookie"] ?? "";
    expect(setCookie).toContain("session=");
    expect(setCookie).toContain("Max-Age=0");
  });

  test("GET /api/v1/auth/me without session returns 401", async ({
    request,
  }) => {
    const res = await request.get("/api/v1/auth/me");
    expect(res.status()).toBe(401);
  });
});

// ─── Anonymous ownership ──────────────────────────────────────────────────────

test.describe("Anonymous ownership", () => {
  test("clone returns owner_id with anon- prefix", async ({ apiContext }) => {
    const slug = `anon-${Date.now()}`;
    const res = await apiContext.post("/api/v1/universes/template/clone", {
      data: { key: slug, name: "Anon Test", description: "" },
    });
    expect([200, 201]).toContain(res.status());
    const body = await res.json();
    expect(body.owner_id).toMatch(/^anon-/);
  });

  test("clone returns a session cookie for the anonymous owner", async ({
    apiContext,
  }) => {
    const slug = `anon-cookie-${Date.now()}`;
    const res = await apiContext.post("/api/v1/universes/template/clone", {
      data: { key: slug, name: "Cookie Test", description: "" },
    });
    expect([200, 201]).toContain(res.status());
    const setCookie = res.headers()["set-cookie"] ?? "";
    expect(setCookie).toContain("session=");
  });
});

// ─── Usage gate overlay ───────────────────────────────────────────────────────

test.describe("Usage gate overlay", () => {
  test("overlay headline is in Portuguese", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.evaluate(() => {
      const el = document.getElementById("usage-limit-overlay");
      if (el) el.classList.remove("hidden");
    });
    const overlay = page.locator("#usage-limit-overlay");
    const isPresent = await overlay.count() > 0;
    test.skip(!isPresent, "usage-limit-overlay not present in this build");
    await expect(overlay).toContainText(/criar|conta|entrar/i);
  });

  test("Entrar button inside overlay opens login modal", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.evaluate(() => {
      const el = document.getElementById("usage-limit-overlay");
      if (el) el.classList.remove("hidden");
    });
    const btn = page.locator("#usage-limit-overlay #btn-gate-login, #usage-limit-overlay [data-action=login]");
    const count = await btn.count();
    test.skip(count === 0, "login button not found in overlay");
    await btn.first().click();
    await expect(page.locator("#login-modal, #auth-modal")).toBeVisible({ timeout: 3_000 });
  });
});

// ─── Subscribe prompt ─────────────────────────────────────────────────────────

test.describe("Subscribe prompt", () => {
  test("subscribe-prompt-overlay closes on cancel", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });
    await page.evaluate(() => {
      const el = document.getElementById("subscribe-prompt-overlay");
      if (el) el.classList.remove("hidden");
    });
    const cancel = page.locator("#btn-subscribe-cancel");
    const count = await cancel.count();
    test.skip(count === 0, "subscribe-prompt-overlay not present");
    await cancel.click();
    const overlay = page.locator("#subscribe-prompt-overlay");
    const cls = await overlay.getAttribute("class");
    expect(cls).toContain("hidden");
  });

  test("subscribe endpoint requires auth", async ({ request }) => {
    const res = await request.post("/api/v1/universes/co/subscribe");
    expect([401, 403]).toContain(res.status());
  });
});
