import { test as base, expect, APIRequestContext } from "@playwright/test";

// --- Types ---

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
  /** A fresh test project created via API before each test */
  seedProject: SeedProject;
}

// --- Fixtures ---

export const test = base.extend<TestFixtures>({
  /** API context pointed at the base URL */
  apiContext: async ({ playwright }, use) => {
    const ctx = await playwright.request.newContext({
      baseURL: "http://localhost:3000",
    });
    await use(ctx);
    await ctx.dispose();
  },

  /** UI variant — override per-test with test.use({ variant: "b" }) */
  variant: ["a", { option: true }],

  /** Creates a uniquely-keyed project via API, tears it down after */
  seedProject: async ({ apiContext }, use) => {
    const suffix = Math.random().toString(36).slice(2, 6).toUpperCase();
    const key = `T${suffix}`;
    const project: SeedProject = {
      name: `Test Project ${key}`,
      key,
      description: "Auto-created by E2E fixture",
    };

    const res = await apiContext.post("/api/projects", { data: project });
    expect(res.status()).toBe(201);

    await use(project);

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
