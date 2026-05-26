/**
 * CO-275 — agent session events: footer on kanban cards
 *
 * Verifies:
 * 1. POST /api/v1/agent/sessions persists a session record (auth required).
 * 2. GET /api/v1/agent/sessions/latest returns the session.
 * 3. Kanban card for a task with a session shows the agent-session footer.
 */

import { test, expect } from "./fixtures";

const TASK_KEY = "CO-275-test";

test.describe("CO-275: agent session events", () => {
  test("POST session requires auth (returns 401 without token)", async ({
    apiContext,
  }) => {
    const res = await apiContext.post("/api/v1/agent/sessions", {
      data: {
        task_id: TASK_KEY,
        universe_key: "co",
        started_at: 1_700_000_000,
        finished_at: 1_700_000_900,
        duration_ms: 900_000,
        exit_code: 0,
      },
    });
    expect(res.status()).toBe(401);
  });

  test("GET sessions/latest returns null for unknown task", async ({
    apiContext,
  }) => {
    const res = await apiContext.get(
      `/api/v1/agent/sessions/latest?task_id=CO-NONEXISTENT-99999`,
    );
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body).toBeNull();
  });

  test("GET sessions returns empty list for unknown task", async ({
    apiContext,
  }) => {
    const res = await apiContext.get(
      `/api/v1/agent/sessions?task_id=CO-NONEXISTENT-99999`,
    );
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.total).toBe(0);
    expect(Array.isArray(body.sessions)).toBe(true);
    expect(body.sessions.length).toBe(0);
  });

  test("kanban card renders agent-session footer when session exists for task", async ({
    page,
    apiContext,
  }) => {
    // Seed a session for one of the real CO dev tasks present in the board
    // by posting with the admin UAT token. We use task CO-272 which is
    // always in the 'done' column after CO-272 was merged.

    // First find a task that exists on the kanban via the dev-tasks API
    const tasksRes = await apiContext.get(
      "/api/v1/universes/co/dev-tasks?limit=5",
    );
    // If the endpoint doesn't exist (not a full server), skip gracefully
    if (tasksRes.status() !== 200) {
      test.skip();
      return;
    }
    const tasksBody = await tasksRes.json();
    if (!tasksBody.tasks || tasksBody.tasks.length === 0) {
      test.skip();
      return;
    }

    const firstTask = tasksBody.tasks[0];
    const taskKey: string = firstTask.key;

    // Login to get a session token for POSTing
    const loginRes = await apiContext.post("/api/v1/auth/uat-login", {
      data: { email: "yuri@uat.local", password: "uat" },
    });
    if (loginRes.status() !== 200) {
      // UAT login not available — skip the write half
      test.skip();
      return;
    }

    // Use cookie-based session from login
    const sessionToken = loginRes.headers()["set-cookie"]
      ?.match(/session=([^;]+)/)?.[1];

    if (!sessionToken) {
      test.skip();
      return;
    }

    // POST a fake session for this task
    const postRes = await apiContext.post("/api/v1/agent/sessions", {
      headers: { Authorization: `Bearer ${sessionToken}` },
      data: {
        task_id: taskKey,
        universe_key: "co",
        started_at: 1_700_000_000,
        finished_at: 1_700_000_840,
        duration_ms: 840_000,
        exit_code: 0,
        tokens_in: 18000,
        tokens_out: 5000,
        tool_calls: { Read: 12, Edit: 6, Bash: 4 },
        skills_loaded: ["rust-architecture"],
        context_chars: 28000,
        final_commit_sha: "5099bd6",
        pr_number: 89,
        model: "sonnet",
        co_auto_version: "0.1.0",
      },
    });
    expect(postRes.status()).toBe(201);

    // Verify retrieval
    const latestRes = await apiContext.get(
      `/api/v1/agent/sessions/latest?task_id=${encodeURIComponent(taskKey)}`,
    );
    expect(latestRes.status()).toBe(200);
    const session = await latestRes.json();
    expect(session).not.toBeNull();
    expect(session.task_id).toBe(taskKey);
    expect(session.exit_code).toBe(0);

    // Visit the /co kanban and check the footer loads
    await page.goto("/co", { waitUntil: "networkidle" });

    // Wait for kanban cards to render
    await expect(page.locator(".task-card").first()).toBeVisible({
      timeout: 15_000,
    });

    // Give lazy-load fetches time to complete
    await page.waitForTimeout(2_000);

    // Assert at least one card has a loaded agent-session footer
    const footers = page.locator(".agent-session-footer.loaded");
    const footerCount = await footers.count();
    expect(footerCount).toBeGreaterThanOrEqual(1);
  });
});
