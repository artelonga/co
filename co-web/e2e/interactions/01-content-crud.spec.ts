import { test, expect, request, APIRequestContext } from "@playwright/test";

/**
 * INTERACTION-01..04: Content CRUD primitives
 *
 * The atomic interactions are the four CRUD operations on a content
 * entry. Content (paths, bodies, frontmatter) is RUNTIME DATA — this
 * test passes opaque fixture data through each primitive to verify
 * the contract. No business meaning is implied by the fixture
 * values.
 *
 * Each operation has its own GIVEN/WHEN/THEN block from
 * registry.yaml. This spec runs the full cycle so a failure points
 * at the broken primitive by name:
 *
 *   1. entryWrite   → PUT entry, then expect 200 on read
 *   2. entryRead    → GET entry, expect body + path match
 *   3. entryList    → GET listing, expect created entry present
 *   4. entryDelete  → DELETE entry, expect 404 on read after
 *
 * SAFETY: the fixture entry lives under a sandbox path
 * `e2e/sandbox/<random-id>.md` so even if cleanup is skipped the
 * write doesn't collide with any user-authored content. Cleanup in
 * `afterEach` deletes it idempotently.
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

test.describe("Content CRUD primitives (entryWrite/Read/List/Delete)", () => {
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

  test("entryWrite → entryRead → entryList → entryWrite (update) → entryDelete", async () => {
    const url = `/api/v1/universes/${encodeURIComponent(TARGET_UNIVERSE)}/entries/${encodePath(sandboxPath)}`;

    // --- 01. entryWrite (create) ------------------------------------------
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
      "entryWrite (create): PUT returns < 400"
    ).toBeLessThan(400);

    // --- 02. entryRead ----------------------------------------------------
    const readRes = await ctx.get(url);
    expect(readRes.status(), "entryRead: GET returns 200").toBe(200);
    const readJson = await readRes.json();
    expect(
      readJson.body,
      "entryRead postcondition: body matches what entryWrite sent"
    ).toBe(initialBody);
    expect(
      readJson.path,
      "entryRead postcondition: response.path === request path"
    ).toBe(sandboxPath);
    // Frontmatter preservation — keys we sent should be present.
    const readFm = readJson.frontmatter ?? {};
    expect(
      readFm.type,
      "entryWrite postcondition: frontmatter.type preserved"
    ).toBe(initialFrontmatter.type);
    expect(
      readFm.title,
      "entryWrite postcondition: frontmatter.title preserved"
    ).toBe(initialFrontmatter.title);

    // --- 03. entryList ----------------------------------------------------
    const listRes = await ctx.get(
      `/api/v1/universes/${encodeURIComponent(TARGET_UNIVERSE)}/entries?limit=500`
    );
    expect(listRes.status(), "entryList: GET returns 200").toBe(200);
    const listJson = await listRes.json();
    const entries: Array<{ path: string; title?: string }> =
      listJson.entries ?? listJson ?? [];
    expect(
      Array.isArray(entries),
      "entryList postcondition: entries is an array"
    ).toBe(true);
    expect(
      entries.some((e) => e.path === sandboxPath),
      "entryList postcondition: sandbox entry appears in listing"
    ).toBe(true);
    if (entries.length > 0) {
      for (const e of entries.slice(0, 10)) {
        expect(
          typeof e.path,
          "entryList postcondition: each item has a path"
        ).toBe("string");
      }
    }

    // --- 01. entryWrite (update) — same primitive, second invocation ------
    const updatedBody = initialBody + "\n\nappended " + Date.now();
    const updateRes = await ctx.put(url, {
      headers: { "content-type": "application/json" },
      data: { body: updatedBody, frontmatter: initialFrontmatter },
    });
    expect(
      updateRes.status(),
      "entryWrite (update): PUT returns < 400"
    ).toBeLessThan(400);
    const readAfterUpdate = await ctx.get(url);
    expect(
      (await readAfterUpdate.json()).body,
      "entryWrite (update) postcondition: body reflects new content"
    ).toBe(updatedBody);

    // --- 04. entryDelete --------------------------------------------------
    const delRes = await ctx.delete(url);
    expect(
      delRes.status(),
      "entryDelete: DELETE returns < 400"
    ).toBeLessThan(400);
    const readAfterDelete = await ctx.get(url);
    expect(
      readAfterDelete.status(),
      "entryDelete postcondition: subsequent entryRead returns 404"
    ).toBe(404);

    // Mark cleanup as a no-op — entry is already gone.
    sandboxPath = "";
  });
});
