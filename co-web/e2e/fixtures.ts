import { test as base, expect, APIRequestContext } from "@playwright/test";

// --- Types ---

export interface TaskShape {
  title: string;
  status?: string;
  priority?: string;
  description?: string;
  labels?: string[];
  due_date?: string;
}

export interface SeedProject {
  name: string;
  key: string;
  description: string;
}

export interface TestFixtures {
  /** Pre-configured API context for direct REST calls */
  apiContext: APIRequestContext;
  /** Current UI variant (a-h), defaults to "a" */
  variant: string;
  /** A fresh test project created via API before each test.
   *  Also authenticates `page` as yuri so board tests can see the project
   *  in the sidebar (the project is created inside the shared e2e-test universe). */
  seedProject: SeedProject & {
    /** Seed additional tasks into the project after creation */
    seedTasks: (tasks: TaskShape[]) => Promise<void>;
  };
}

// --- Fixtures ---

export const test = base.extend<TestFixtures>({
  /** API context pointed at the base URL, authenticated as the test admin.
   *  Server boots with CO_ENV=test (see e2e/global-setup.ts) which enables
   *  uat-login + seeds yuri@uat.local. The login response sets a session
   *  cookie on the context — subsequent requests carry it automatically. */
  apiContext: async ({ playwright }, use) => {
    const baseURL = process.env.BASE_URL ?? "http://localhost:3000";
    const ctx = await playwright.request.newContext({ baseURL });
    const loginRes = await ctx.post("/api/v1/auth/uat-login", {
      data: { email: "yuri@uat.local", password: "uat" },
    });
    if (!loginRes.ok()) {
      throw new Error(
        `apiContext fixture: uat-login failed (${loginRes.status()}). ` +
          `Server must boot with CO_ENV=test or CO_ENV=uat to enable this path.`,
      );
    }
    await use(ctx);
    await ctx.dispose();
  },

  /** UI variant — override per-test with test.use({ variant: "b" }) */
  variant: ["a", { option: true }],

  /** Creates a uniquely-keyed project inside the shared `e2e-test` universe and
   *  authenticates the `page` browser context as yuri so the board tests can find
   *  the project in the authenticated sidebar.
   *
   *  Also provides `seedTasks(tasks)` to pre-populate tasks without relying on
   *  dirty test-database contents:
   *
   *  ```ts
   *  test("drag a card", async ({ page, seedProject }) => {
   *    await seedProject.seedTasks([{ title: "Drag me", status: "todo" }]);
   *    await navigateTo(page, "/");
   *    await selectProject(page, seedProject.key);
   *    // ...
   *  });
   *  ```
   */
  seedProject: async ({ apiContext, page }, use) => {
    // Authenticate the page browser context.
    // page.request shares the browser context cookie jar — the session cookie
    // returned by uat-login is stored and sent with subsequent page.goto() calls.
    await page.request.post("/api/v1/auth/uat-login", {
      data: { email: "yuri@uat.local", password: "uat" },
    });

    // Guarantee the SPA boots the e2e-test universe at "/" rather than the first
    // entry in yuri's owned list (which may be anon-clone universes created by
    // other test suites in the same DB session).  addInitScript() runs before
    // any page script so localStorage is set before init() reads it.
    await page.addInitScript(() => {
      try { localStorage.setItem("co_preferred_universe", "e2e-test"); } catch (_) {}
    });

    const suffix = Math.random().toString(36).slice(2, 6).toUpperCase();
    const key = `T${suffix}`;
    const project: SeedProject = {
      name: `Test Project ${key}`,
      key,
      description: "Auto-created by E2E fixture",
    };

    // Create in the shared e2e-test universe so it appears in yuri's sidebar.
    const res = await apiContext.post("/api/projects", {
      data: { ...project, universe_key: "e2e-test" },
    });
    expect(res.status()).toBe(201);

    const seedTasks = async (tasks: TaskShape[]) => {
      for (const task of tasks) {
        const body: Record<string, unknown> = {
          title: task.title,
          description: task.description ?? "",
          status: task.status ?? "todo",
          priority: task.priority ?? "medium",
          labels: task.labels ?? [],
        };
        if (task.due_date) body.due_date = task.due_date;
        const r = await apiContext.post(`/api/projects/${key}/tasks`, {
          data: body,
        });
        expect(r.status()).toBe(201);
      }
    };

    await use({ ...project, seedTasks });

    // Cleanup: delete all tasks then we leave the project
    // (no delete-project endpoint, so this is best-effort)
    const tasksRes = await apiContext.get(`/api/projects/${key}/tasks`);
    if (tasksRes.ok()) {
      const tasks = await tasksRes.json();
      for (const t of tasks) {
        await apiContext.delete(`/api/projects/${key}/tasks/${t.id}`);
      }
    }
  },
});

export { expect };
