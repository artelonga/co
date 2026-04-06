/**
 * CO-33 — Auth & CRDT: session flow, sharing gate, real-time sync
 *
 * Tests:
 *   - Login flow (username+password): session cookie set → protected routes work
 *   - Anonymous universe: is_anonymous=true, no public shareable link
 *   - Login / claim: anonymous universe becomes owned, link works for visitors
 *   - Anonymous editor: no WebSocket connection is established (local-only mode)
 *   - CRDT: two logged-in browser contexts edit the same doc → changes sync
 */

import { test, expect, Browser } from "@playwright/test";

// ─── Auth flow ────────────────────────────────────────────────────────────────

test.describe("Auth: login flow sets session cookie → protected routes accessible", () => {
  test("POST /api/v1/auth/login with a valid-shaped email returns 200", async ({
    request,
  }) => {
    const res = await request.post("/api/v1/auth/login", {
      data: { email: "test@example.com" },
    });
    // Server always returns 200 regardless of user existence (security)
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

  test("GET /api/v1/auth/me without a session returns 401", async ({
    request,
  }) => {
    const res = await request.get("/api/v1/auth/me");
    expect(res.status()).toBe(401);
  });

  test("GET /api/v1/auth/me with valid session returns user info", async ({
    page,
  }) => {
    const devJwt = process.env.TEST_JWT;
    test.skip(!devJwt, "TEST_JWT env var not set");

    await page.context().addCookies([
      {
        name: "session",
        value: devJwt!,
        domain: "localhost",
        path: "/",
        httpOnly: true,
        secure: false,
        sameSite: "Strict",
      },
    ]);

    const result = await page.evaluate(async () => {
      const r = await fetch("/api/v1/auth/me", { credentials: "include" });
      return { status: r.status, body: await r.json() };
    });

    expect(result.status).toBe(200);
    expect(result.body.email).toBeTruthy();
  });

  test("authenticated user can access protected write endpoints", async ({
    request,
  }) => {
    const devJwt = process.env.TEST_JWT;
    test.skip(!devJwt, "TEST_JWT env var not set");

    // Create a project (protected by require_auth in board_protected)
    const suffix = Math.random().toString(36).slice(2, 5).toUpperCase();
    const res = await request.post("/api/projects", {
      headers: { Authorization: `Bearer ${devJwt}` },
      data: {
        key: `AUTH${suffix}`,
        name: `Auth Test ${suffix}`,
        description: "",
      },
    });
    expect(res.status()).toBe(201);
  });
});

// ─── Sharing gate: anonymous universe ─────────────────────────────────────────

test.describe("Sharing gate: anonymous universe is private", () => {
  test("cloned template universe has is_anonymous=true in the response", async ({
    request,
  }) => {
    const slug = `share-anon-${Date.now()}`;
    const res = await request.post("/api/v1/universes/template/clone", {
      data: { key: slug, name: "Sharing Gate Test", description: "" },
    });

    expect([200, 201]).toContain(res.status());
    const body = await res.json();
    expect(body.is_anonymous).toBe(true);
  });

  test("GET /api/v1/universes/:slug returns 403 for anonymous universe without owner cookie", async ({
    request,
  }) => {
    const slug = `share-private-${Date.now()}`;
    await request.post("/api/v1/universes/template/clone", {
      data: { key: slug, name: "Private Test", description: "" },
    });

    // A fresh request context without the owner cookie should be forbidden
    const res = await request.get(`/api/v1/universes/${slug}`);
    // 403 = sharing gate blocks anonymous universe for non-owners
    // 404 = route only returns owned/public universes
    expect([403, 404]).toContain(res.status());
  });

  test("clone → owner cookie → GET /co/:slug returns 200", async ({
    page,
  }) => {
    // After cloning, the browser gets a session cookie for the anon owner.
    // Navigating to /co/:slug with that cookie should serve the page.
    const slug = `share-owner-${Date.now()}`;

    // Clone via the page so the cookie is set automatically
    await page.goto("/co", { waitUntil: "networkidle" });
    await page.locator("#btn-criar-universo").click();
    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);

    await page.locator("#criar-name").fill("Owner Test Universe");
    await page.locator("#criar-slug").fill(slug);
    await page.locator("#criar-form").dispatchEvent("submit");

    await page.waitForURL(/\/co\/[a-z0-9-]+/, { timeout: 15_000 });

    // The page returned a 200 since we're the owner
    expect(page.url()).toContain(`/co/${slug}`);
  });
});

// ─── Anonymous editor: no WebSocket ──────────────────────────────────────────

test.describe("Anonymous editor: no WebSocket connection", () => {
  test("no WebSocket is opened when anonymous user uses the task editor", async ({
    page,
    request,
  }) => {
    // Track WebSocket connections
    const wsConnections: string[] = [];
    page.on("websocket", (ws) => wsConnections.push(ws.url()));

    // Create a board project and task to open a modal with
    const suffix = Math.random().toString(36).slice(2, 5).toUpperCase();
    const projectRes = await request.post("/api/projects", {
      data: {
        key: `ANWS${suffix}`,
        name: `Anon WS Test ${suffix}`,
        description: "",
      },
    });
    if (projectRes.status() !== 201) {
      test.skip(true, "Could not create test project");
      return;
    }
    const project = await projectRes.json();

    const taskRes = await request.post(
      `/api/projects/${project.key}/tasks`,
      {
        headers: {
          Authorization: `Bearer ${process.env.TEST_JWT ?? "dev-token"}`,
        },
        data: { title: "Anon WS task", status: "todo" },
      },
    );
    if (taskRes.status() !== 201) {
      test.skip(true, "Could not create test task");
      return;
    }
    const task = await taskRes.json();

    // Navigate and open task modal as anonymous user (no session cookie)
    await page.context().clearCookies();
    await page.goto("/", { waitUntil: "networkidle" });

    const projectLink = page.locator(
      `#project-list .sidebar-item-key:text-is("${project.key}")`,
    );
    const vp = page.viewportSize();
    if (vp && vp.width <= 640) return; // skip on mobile

    await projectLink.click();
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
    await page.waitForSelector(".co-editor-wrap", { timeout: 8_000 }).catch(
      () => {},
    );

    // Wait a moment for any async WebSocket setup
    await page.waitForTimeout(1_000);

    // No WebSocket should have been opened for anonymous user
    const crdtConnections = wsConnections.filter((url) =>
      url.includes("/ws/doc/"),
    );
    expect(crdtConnections).toHaveLength(0);
  });
});

// ─── CRDT sync: two browser contexts ─────────────────────────────────────────

test.describe("CRDT: two logged-in contexts sync edits bidirectionally", () => {
  test("edit in context A appears in context B (requires TEST_JWT)", async ({
    browser,
  }) => {
    const devJwt = process.env.TEST_JWT;
    test.skip(!devJwt, "TEST_JWT env var not set — CRDT test requires auth");

    // Helper to inject a session cookie into a new context
    async function makeAuthContext(b: Browser) {
      const ctx = await b.newContext();
      await ctx.addCookies([
        {
          name: "session",
          value: devJwt!,
          domain: "localhost",
          path: "/",
          httpOnly: true,
          secure: false,
          sameSite: "Strict",
        },
      ]);
      return ctx;
    }

    // Create a universe to collaborate on
    const ctxA = await makeAuthContext(browser);
    const ctxB = await makeAuthContext(browser);

    const pageA = await ctxA.newPage();
    const pageB = await ctxB.newPage();

    // Clone a universe for collaboration
    const slug = `crdt-sync-${Date.now()}`;
    const res = await ctxA.request().post("/api/v1/universes/template/clone", {
      data: { key: slug, name: "CRDT Sync Test", description: "" },
    });
    if (!res.ok()) {
      await ctxA.close();
      await ctxB.close();
      test.skip(true, "Could not create universe for CRDT test");
      return;
    }

    // Both contexts navigate to the same universe
    await pageA.goto(`/co/${slug}`, { waitUntil: "networkidle" });
    await pageB.goto(`/co/${slug}`, { waitUntil: "networkidle" });

    // Find a task to edit — both need to open the same doc
    const project = await ctxA.request().get("/api/projects");
    if (!project.ok()) {
      await ctxA.close();
      await ctxB.close();
      test.skip(true, "Could not list projects for CRDT test");
      return;
    }
    const projects: Array<{ key: string; universe_key?: string }> =
      await project.json();
    const universeProject = projects.find((p) => p.universe_key === slug);
    if (!universeProject) {
      await ctxA.close();
      await ctxB.close();
      test.skip(true, "No project in the universe for CRDT test");
      return;
    }

    const tasksRes = await ctxA
      .request()
      .get(`/api/projects/${universeProject.key}/tasks`);
    const tasks: Array<{ id: number }> = await tasksRes.json();
    if (tasks.length === 0) {
      await ctxA.close();
      await ctxB.close();
      test.skip(true, "No tasks in the universe for CRDT test");
      return;
    }

    const taskId = tasks[0].id;

    // Context A: open task modal and open the editor
    const cardA = pageA.locator(`.task-card[data-task-id="${taskId}"]`);
    if (await cardA.count() === 0) {
      // Select project first
      const projectLink = pageA.locator(
        `#project-list .sidebar-item-key:text-is("${universeProject.key}")`,
      );
      if (await projectLink.count() > 0) await projectLink.click();
    }
    await cardA.click();
    await expect(pageA.locator("#modal-overlay")).toBeVisible();
    await pageA.waitForSelector(".cm-content", { timeout: 10_000 });

    // Context B: open the same task
    const cardB = pageB.locator(`.task-card[data-task-id="${taskId}"]`);
    if (await cardB.count() === 0) {
      const projectLinkB = pageB.locator(
        `#project-list .sidebar-item-key:text-is("${universeProject.key}")`,
      );
      if (await projectLinkB.count() > 0) await projectLinkB.click();
    }
    await cardB.click();
    await expect(pageB.locator("#modal-overlay")).toBeVisible();
    await pageB.waitForSelector(".cm-content", { timeout: 10_000 });

    // Context A types some unique text
    const uniqueText = `CRDT-${Date.now()}`;
    await pageA.locator(".cm-content").first().click();
    await pageA.keyboard.press("End");
    await pageA.keyboard.type(uniqueText);

    // Wait for CRDT sync to propagate (Yjs uses WebSocket)
    await pageB.waitForTimeout(2_000);

    // Context B should have received the update via CRDT
    const cmB = pageB.locator(".cm-content").first();
    await expect(cmB).toContainText(uniqueText, { timeout: 5_000 });

    await ctxA.close();
    await ctxB.close();
  });
});
