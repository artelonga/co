/**
 * CO-33 — Usage gate: 100 entries free, 101st prompts account creation
 *
 * The usage gate fires when an anonymous universe's content_count reaches 100.
 * The server returns HTTP 402 with `{"error":"usage_limit","current":N}`.
 * The frontend responds by showing `#usage-limit-overlay`.
 *
 * Tests:
 *   - API contract: 402 response structure is correct
 *   - DOM: #usage-limit-overlay shows correct copy and action buttons
 *   - UX: clicking "Entrar" on the gate overlay opens the login modal
 *   - UX: language toggle on the gate overlay works
 */

import { test, expect } from "./fixtures";

// ─── API contract ─────────────────────────────────────────────────────────────

test.describe("Usage gate: API contract", () => {
  test("clone template → universe is created with anonymous ownership", async ({
    apiContext,
  }) => {
    const slug = `gate-anon-${Date.now()}`;
    const res = await apiContext.post("/api/v1/universes/template/clone", {
      data: { key: slug, name: "Gate Anon Test", description: "" },
    });

    // Either 200 or 201
    expect([200, 201]).toContain(res.status());
    const body = await res.json();
    expect(body.key).toBe(slug);
    // is_anonymous was removed in CO-170/CO-184; anonymous ownership is now
    // indicated by owner_id starting with "anon-".
    expect(body.owner_id).toMatch(/^anon-/);
  });

  test("clone returns a session cookie for the anonymous owner", async ({
    apiContext,
  }) => {
    const slug = `gate-cookie-${Date.now()}`;
    const res = await apiContext.post("/api/v1/universes/template/clone", {
      data: { key: slug, name: "Cookie Test", description: "" },
    });

    expect([200, 201]).toContain(res.status());
    const setCookie = res.headers()["set-cookie"] ?? "";
    expect(setCookie).toContain("session=");
  });

  test("GET /api/v1/universes/:slug reports content_count", async ({
    apiContext,
  }) => {
    const slug = `gate-count-${Date.now()}`;
    await apiContext.post("/api/v1/universes/template/clone", {
      data: { key: slug, name: "Count Test", description: "" },
    });

    const res = await apiContext.get(`/api/v1/universes/${slug}`);
    // Either 200 (public/template clone) or 404 if not publicly accessible
    if (res.status() === 200) {
      const body = await res.json();
      expect(typeof body.content_count).toBe("number");
    }
  });
});

// ─── DOM: overlay structure ───────────────────────────────────────────────────

test.describe("Usage gate: overlay DOM structure and copy", () => {
  test("usage-limit-overlay contains the expected headline and message", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    // Force the overlay visible to test its content
    await page.evaluate(() => {
      const overlay = document.getElementById("usage-limit-overlay");
      if (overlay) overlay.classList.remove("hidden");
    });

    await expect(page.locator("#usage-limit-overlay")).not.toHaveClass(/hidden/);
    await expect(page.locator("#usage-limit-title")).toBeVisible();
    await expect(page.locator("#usage-limit-msg")).toBeVisible();
    // Default Portuguese text
    await expect(page.locator("#usage-limit-title")).toContainText(
      /Limite atingido/i,
    );
  });

  test("usage-limit-overlay shows 'Criar conta' and 'Entrar' buttons", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    await page.evaluate(() => {
      const overlay = document.getElementById("usage-limit-overlay");
      if (overlay) overlay.classList.remove("hidden");
    });

    await expect(page.locator("#btn-usage-register")).toBeVisible();
    await expect(page.locator("#btn-usage-login")).toBeVisible();
  });
});

// ─── UX interactions ──────────────────────────────────────────────────────────

test.describe("Usage gate: UX after gate fires", () => {
  test("clicking 'Entrar' on the gate overlay opens the login modal", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    // Show the usage gate overlay
    await page.evaluate(() => {
      const overlay = document.getElementById("usage-limit-overlay");
      if (overlay) overlay.classList.remove("hidden");
    });

    await expect(page.locator("#usage-limit-overlay")).not.toHaveClass(/hidden/);
    await page.locator("#btn-usage-login").click();

    // Login modal should open
    await expect(
      page.locator("#login-modal-overlay"),
    ).not.toHaveClass(/hidden/);
  });

  test("language toggle on the gate overlay switches language", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    await page.evaluate(() => {
      const overlay = document.getElementById("usage-limit-overlay");
      if (overlay) overlay.classList.remove("hidden");
    });

    const langBtn = page.locator("#btn-usage-lang");
    await expect(langBtn).toBeVisible();

    // Default is pt — toggle button shows "English"
    const beforeText = await langBtn.textContent();

    await langBtn.click();
    await page.waitForTimeout(300); // allow i18n update

    const afterText = await langBtn.textContent();
    expect(afterText).not.toBe(beforeText);
  });

  test("API returns 402 with usage_limit error structure when gate fires", async ({
    apiContext,
  }) => {
    // Clone an anon universe
    const slug = `gate-402-${Date.now()}`;
    const cloneRes = await apiContext.post("/api/v1/universes/template/clone", {
      data: { key: slug, name: "Gate 402 Test", description: "" },
    });
    expect([200, 201]).toContain(cloneRes.status());

    // Extract the session cookie for the anonymous owner
    const setCookie = cloneRes.headers()["set-cookie"] ?? "";
    const sessionMatch = setCookie.match(/session=([^;]+)/);
    const anonToken = sessionMatch?.[1] ?? "";

    if (!anonToken) {
      // Skip if we can't get the session cookie
      test.skip(true, "Could not extract anon session cookie");
      return;
    }

    // Get existing projects in the cloned universe to find a project key
    const projectsRes = await apiContext.get("/api/projects", {
      headers: { Cookie: `session=${anonToken}` },
    });
    if (!projectsRes.ok()) {
      test.skip(true, "Could not list projects for anon universe");
      return;
    }
    const projects: Array<{ key: string; universe_key?: string }> =
      await projectsRes.json();
    const universeProject = projects.find((p) => p.universe_key === slug);
    if (!universeProject) {
      test.skip(true, "No project found for the cloned universe");
      return;
    }

    const projectKey = universeProject.key;

    // Create tasks until we get a 402 (max 110 attempts)
    let gotGate = false;
    for (let i = 0; i < 110; i++) {
      const res = await apiContext.post(
        `/api/projects/${projectKey}/tasks`,
        {
          headers: { Cookie: `session=${anonToken}` },
          data: { title: `Flood task ${i}`, status: "todo" },
        },
      );
      if (res.status() === 402) {
        const body = await res.json();
        expect(body.error).toBe("usage_limit");
        expect(typeof body.current).toBe("number");
        gotGate = true;
        break;
      }
      if (res.status() !== 201 && res.status() !== 200) break;
    }
    // We expect to have hit the gate within 110 attempts given a fresh clone
    expect(gotGate).toBe(true);
  });
});
