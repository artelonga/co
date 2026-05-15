import { test, expect, request, APIRequestContext } from "@playwright/test";

/**
 * Content interactions — full CRUD cycle on the `entry` resource.
 *
 * The atomic interaction is one resource (`entry`) with four
 * operations defined by HTTP verb. Operation IDs match the canonical
 * OpenAPI 3.1 doc in `registry.yaml`:
 *
 *   putEntry      PUT    /api/v1/universes/{universe}/entries/{path}
 *   getEntry      GET    /api/v1/universes/{universe}/entries/{path}
 *   deleteEntry   DELETE /api/v1/universes/{universe}/entries/{path}
 *   listEntries   GET    /api/v1/universes/{universe}/entries
 *
 * `{universe}` is a path parameter — the spec runs unchanged
 * against any universe by setting `CO_TEST_UNIVERSE`.
 *
 * Content (paths, bodies, frontmatter) is RUNTIME DATA — fixtures
 * here carry no business meaning. The test verifies the
 * postconditions documented as `x-postconditions` in registry.yaml.
 *
 * SAFETY: the fixture entry lives at `e2e/sandbox/<random>.md` so a
 * skipped cleanup can never overwrite user content. `afterEach`
 * deletes idempotently.
 */

const BASE = process.env.BASE_URL ?? "https://co-artelonga.fly.dev";
const USER_EMAIL = process.env.CO_TEST_USER_EMAIL ?? "";
const USER_PASSWORD = process.env.CO_TEST_USER_PASSWORD ?? "";
const TARGET_UNIVERSE = process.env.CO_TEST_UNIVERSE ?? "artelonga";

async function authenticate(ctx: APIRequestContext): Promise<void> {
  const res = await ctx.post("/api/v1/auth/password-login", {
    headers: { "content-type": "application/json" },
    data: { email: USER_EMAIL, password: USER_PASSWORD },
  });
  expect(res.status(), "password-login should succeed").toBe(200);
}

function encodePath(path: string): string {
  return path.split("/").map(encodeURIComponent).join("/");
}

function randomSandboxPath(): string {
  const suffix = Math.random().toString(36).slice(2, 10);
  return `e2e/sandbox/${suffix}.md`;
}

test.describe("entry resource — full CRUD cycle", () => {
  test.skip(
    !USER_EMAIL || !USER_PASSWORD,
    "Set CO_TEST_USER_EMAIL + CO_TEST_USER_PASSWORD to run."
  );

  let ctx: APIRequestContext;
  let sandboxPath = "";

  test.beforeEach(async () => {
    ctx = await request.newContext({
      baseURL: BASE,
      ignoreHTTPSErrors: true,
    });
    await authenticate(ctx);
    sandboxPath = randomSandboxPath();
  });

  test.afterEach(async () => {
    if (sandboxPath) {
      try {
        await ctx.delete(
          `/api/v1/universes/${encodeURIComponent(TARGET_UNIVERSE)}/entries/${encodePath(sandboxPath)}`
        );
      } catch (_) {
        /* best-effort */
      }
    }
    await ctx.dispose();
  });

  test("putEntry → getEntry → listEntries → putEntry (update) → deleteEntry", async () => {
    const url = `/api/v1/universes/${encodeURIComponent(TARGET_UNIVERSE)}/entries/${encodePath(sandboxPath)}`;

    // --- 01. putEntry (create) ------------------------------------------
    const initialBody = "# Sandbox\n\nopaque fixture content " + Date.now();
    const initialFrontmatter = {
      type: "page",
      title: "Sandbox",
      tags: ["e2e", "interaction-01"],
    };
    const writeRes = await ctx.put(url, {
      headers: { "content-type": "application/json" },
      data: { body: initialBody, frontmatter: initialFrontmatter },
    });
    expect(
      writeRes.status(),
      "putEntry (create): PUT returns < 400"
    ).toBeLessThan(400);

    // --- 02. getEntry ----------------------------------------------------
    const readRes = await ctx.get(url);
    expect(readRes.status(), "getEntry: GET returns 200").toBe(200);
    const readJson = await readRes.json();
    expect(
      readJson.body,
      "getEntry postcondition: body matches what putEntry sent"
    ).toBe(initialBody);
    expect(
      readJson.path,
      "getEntry postcondition: response.path === request path"
    ).toBe(sandboxPath);
    // Frontmatter preservation — keys we sent should be present.
    const readFm = readJson.frontmatter ?? {};
    expect(
      readFm.type,
      "putEntry postcondition: frontmatter.type preserved"
    ).toBe(initialFrontmatter.type);
    expect(
      readFm.title,
      "putEntry postcondition: frontmatter.title preserved"
    ).toBe(initialFrontmatter.title);

    // --- 03. listEntries ----------------------------------------------------
    const listRes = await ctx.get(
      `/api/v1/universes/${encodeURIComponent(TARGET_UNIVERSE)}/entries?limit=500`
    );
    expect(listRes.status(), "listEntries: GET returns 200").toBe(200);
    const listJson = await listRes.json();
    const entries: Array<{ path: string; title?: string }> =
      listJson.entries ?? listJson ?? [];
    expect(
      Array.isArray(entries),
      "listEntries postcondition: entries is an array"
    ).toBe(true);
    expect(
      entries.some((e) => e.path === sandboxPath),
      "listEntries postcondition: sandbox entry appears in listing"
    ).toBe(true);
    if (entries.length > 0) {
      for (const e of entries.slice(0, 10)) {
        expect(
          typeof e.path,
          "listEntries postcondition: each item has a path"
        ).toBe("string");
      }
    }

    // --- 01. putEntry (update) — same primitive, second invocation ------
    const updatedBody = initialBody + "\n\nappended " + Date.now();
    const updateRes = await ctx.put(url, {
      headers: { "content-type": "application/json" },
      data: { body: updatedBody, frontmatter: initialFrontmatter },
    });
    expect(
      updateRes.status(),
      "putEntry (update): PUT returns < 400"
    ).toBeLessThan(400);
    const readAfterUpdate = await ctx.get(url);
    expect(
      (await readAfterUpdate.json()).body,
      "putEntry (update) postcondition: body reflects new content"
    ).toBe(updatedBody);

    // --- 04. deleteEntry --------------------------------------------------
    const delRes = await ctx.delete(url);
    expect(
      delRes.status(),
      "deleteEntry: DELETE returns < 400"
    ).toBeLessThan(400);
    const readAfterDelete = await ctx.get(url);
    expect(
      readAfterDelete.status(),
      "deleteEntry postcondition: subsequent getEntry returns 404"
    ).toBe(404);

    // Mark cleanup as a no-op — entry is already gone.
    sandboxPath = "";
  });
});
