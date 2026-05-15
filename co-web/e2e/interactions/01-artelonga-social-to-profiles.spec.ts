import { test, expect, request, APIRequestContext } from "@playwright/test";

/**
 * INTERACTION-01: Switch ArteLonga social links to internal profile wikilinks
 *
 * REF: artelonga::sobre.md  (the universe::path notation; universe = 'artelonga',
 *      path = 'sobre.md')
 *
 * GIVEN:
 *   - The `artelonga` universe exists, is public-subscribable.
 *   - `artelonga::sobre.md` contains markdown links of the shape
 *     `[<handle>](https://www.instagram.com/<handle>/)` for each member
 *     of the editorial board.
 *   - One member (`falcao`) is already referenced as a bare wikilink
 *     `[[falcao]]` pointing to a profile that does not yet exist
 *     anywhere in the artelonga universe.
 *
 * WHEN:
 *   - The user edits `sobre.md` and replaces each Instagram external
 *     URL with an internal wikilink to the corresponding profile
 *     (e.g. `[yuri-sugano](https://www.instagram.com/yvsugano/)` →
 *     `[[yvsugano|yuri-sugano]]`).
 *
 * THEN (acceptance criteria — one assertion per bullet):
 *   1. `sobre.md` body no longer contains the substring
 *      `https://www.instagram.com/` (all external links removed).
 *   2. `sobre.md` body contains wikilinks `[[<handle>]]` or
 *      `[[<handle>|<alias>]]` for each former Instagram handle.
 *   3. The pre-existing `[[falcao]]` wikilink is preserved exactly.
 *   4. A sub-task entry exists at
 *      `artelonga::projects/AL/<next-id>.md` with frontmatter
 *      `type: task`, `status: todo`, and a title referencing
 *      "criar perfil falcao".
 *   5. Both entries (the edited `sobre.md` and the new task) are
 *      visible via the public entries API.
 *
 * SAFETY:
 *   - The original `sobre.md` body is snapshotted before mutation
 *     and restored in `afterEach`.
 *   - The new task entry is deleted in `afterEach`.
 *   - If `CO_TEST_USER_EMAIL` / `CO_TEST_USER_PASSWORD` are not set,
 *     the test is skipped — a CI without secrets should not go red.
 */

const BASE = process.env.BASE_URL ?? "https://co-artelonga.fly.dev";
const USER_EMAIL = process.env.CO_TEST_USER_EMAIL ?? "";
const USER_PASSWORD = process.env.CO_TEST_USER_PASSWORD ?? "";

// ---------------------------------------------------------------------------
// Transformation: instagram external link → internal wikilink
//
//   `[yuri-sugano](https://www.instagram.com/yvsugano/)`
//     → `[[yvsugano|yuri-sugano]]`
//
// The IG handle becomes the wikilink target (canonical profile slug),
// the original visible label becomes the wikilink alias so the
// rendered text doesn't visually change for readers.
// ---------------------------------------------------------------------------
const IG_LINK_RE =
  /\[([^\]]+)\]\(https?:\/\/(?:www\.)?instagram\.com\/([A-Za-z0-9_.]+)\/?\)/g;

function rewriteSocialToProfiles(md: string): string {
  return md.replace(IG_LINK_RE, (_, label, handle) => `[[${handle}|${label}]]`);
}

// ---------------------------------------------------------------------------
// Auth helper — exchanges email/password for a session cookie that
// subsequent requests carry.
// ---------------------------------------------------------------------------
async function authenticate(ctx: APIRequestContext): Promise<void> {
  const res = await ctx.post("/api/v1/auth/password-login", {
    headers: { "content-type": "application/json" },
    data: { email: USER_EMAIL, password: USER_PASSWORD },
  });
  expect(res.status(), "password-login should succeed").toBe(200);
}

// ---------------------------------------------------------------------------
// Find the next free task id under projects/AL/.
// ---------------------------------------------------------------------------
async function nextAlTaskId(ctx: APIRequestContext): Promise<number> {
  const res = await ctx.get(
    "/api/v1/universes/artelonga/entries?type=task&limit=200"
  );
  expect(res.status()).toBe(200);
  const json = await res.json();
  const entries: Array<{ path: string }> = json.entries ?? json ?? [];
  let max = 0;
  for (const e of entries) {
    const m = e.path.match(/^projects\/AL\/(\d+)\.md$/);
    if (m) {
      const n = parseInt(m[1], 10);
      if (n > max) max = n;
    }
  }
  return max + 1;
}

test.describe("INTERACTION-01: ArteLonga social → internal profile wikilinks", () => {
  test.skip(
    !USER_EMAIL || !USER_PASSWORD,
    "Set CO_TEST_USER_EMAIL + CO_TEST_USER_PASSWORD to run this interaction."
  );

  let ctx: APIRequestContext;
  let originalBody = "";
  let newTaskPath = "";

  test.beforeEach(async () => {
    ctx = await request.newContext({
      baseURL: BASE,
      ignoreHTTPSErrors: true,
    });
    await authenticate(ctx);
  });

  test.afterEach(async () => {
    // Restore original sobre.md body
    if (originalBody) {
      try {
        await ctx.put("/api/v1/universes/artelonga/entries/sobre.md", {
          headers: { "content-type": "application/json" },
          data: { body: originalBody },
        });
      } catch (_) {
        /* best-effort cleanup */
      }
    }
    // Delete the new task entry
    if (newTaskPath) {
      try {
        await ctx.delete(
          `/api/v1/universes/artelonga/entries/${newTaskPath
            .split("/")
            .map(encodeURIComponent)
            .join("/")}`
        );
      } catch (_) {
        /* best-effort cleanup */
      }
    }
    await ctx.dispose();
  });

  test("edit sobre.md + create falcao profile task; both open", async () => {
    // GIVEN: snapshot current sobre.md body
    const fetched = await ctx.get(
      "/api/v1/universes/artelonga/entries/sobre.md"
    );
    expect(fetched.status(), "sobre.md must exist").toBe(200);
    const before = await fetched.json();
    originalBody = before.body ?? "";
    expect(
      originalBody.length,
      "precondition: sobre.md has content"
    ).toBeGreaterThan(0);
    expect(
      originalBody.includes("https://www.instagram.com/"),
      "precondition: sobre.md contains at least one Instagram external link"
    ).toBe(true);
    expect(
      originalBody.includes("[[falcao]]"),
      "precondition: bare [[falcao]] wikilink is present"
    ).toBe(true);

    // WHEN: transform Instagram external links → wikilinks
    const newBody = rewriteSocialToProfiles(originalBody);

    const putSobre = await ctx.put(
      "/api/v1/universes/artelonga/entries/sobre.md",
      {
        headers: { "content-type": "application/json" },
        data: { body: newBody },
      }
    );
    expect(putSobre.status(), "PUT sobre.md").toBeLessThan(400);

    // AND: create the sub-task for the missing falcao profile
    const nextId = await nextAlTaskId(ctx);
    newTaskPath = `projects/AL/${nextId}.md`;
    const taskFrontmatter = {
      type: "task",
      id: nextId,
      title: "Criar perfil [[falcao]] em comunidades/",
      status: "todo",
      priority: "medium",
      project_key: "AL",
      tags: ["perfis", "comunidades", "follow-up:interaction-01"],
    };
    const taskBody = [
      "Sub-task disparado pela INTERACTION-01.",
      "",
      "O wikilink `[[falcao]]` em `sobre.md` aponta para um perfil que ainda",
      "não existe no universo `artelonga`. Criar a página:",
      "",
      "- Path sugerido: `comunidades/falcao.md`",
      "- Conteúdo: bio breve, função (skateboard / direção de ESG),",
      "  rede social externa se relevante.",
      "",
      "Ao concluir, fechar este task e marcar a interação 01 como done.",
      "",
    ].join("\n");
    const putTask = await ctx.put(
      `/api/v1/universes/artelonga/entries/${newTaskPath
        .split("/")
        .map(encodeURIComponent)
        .join("/")}`,
      {
        headers: { "content-type": "application/json" },
        data: { body: taskBody, frontmatter: taskFrontmatter },
      }
    );
    expect(putTask.status(), "PUT new task").toBeLessThan(400);

    // THEN — one assertion per acceptance criterion ----------------

    // 1. sobre.md no longer contains Instagram external URLs
    const after = await ctx.get(
      "/api/v1/universes/artelonga/entries/sobre.md"
    );
    const afterBody = (await after.json()).body ?? "";
    expect(
      afterBody.includes("https://www.instagram.com/"),
      "criterion 1: no Instagram URLs remain"
    ).toBe(false);

    // 2. sobre.md contains wikilinks for every former IG handle
    const originalHandles: string[] = [];
    for (const m of originalBody.matchAll(IG_LINK_RE)) {
      originalHandles.push(m[2]);
    }
    expect(originalHandles.length).toBeGreaterThan(0);
    for (const handle of originalHandles) {
      expect(
        new RegExp(`\\[\\[${handle}(?:\\||\\]\\])`).test(afterBody),
        `criterion 2: wikilink [[${handle}]] present`
      ).toBe(true);
    }

    // 3. The pre-existing [[falcao]] wikilink is preserved
    expect(
      afterBody.includes("[[falcao]]"),
      "criterion 3: [[falcao]] preserved"
    ).toBe(true);

    // 4. The new task entry exists with status=todo
    const taskRes = await ctx.get(
      `/api/v1/universes/artelonga/entries/${newTaskPath
        .split("/")
        .map(encodeURIComponent)
        .join("/")}`
    );
    expect(taskRes.status(), "criterion 4a: task entry GET 200").toBe(200);
    const taskEntry = await taskRes.json();
    expect(taskEntry.frontmatter?.type, "criterion 4b: type=task").toBe("task");
    expect(taskEntry.frontmatter?.status, "criterion 4c: status=todo").toBe(
      "todo"
    );
    expect(
      String(taskEntry.frontmatter?.title || "").toLowerCase(),
      "criterion 4d: title references falcao"
    ).toContain("falcao");

    // 5. Both entries appear in the public entries listing
    const list = await ctx.get(
      `/api/v1/universes/artelonga/entries?limit=500`
    );
    const listJson = await list.json();
    const paths: string[] = (listJson.entries ?? listJson ?? []).map(
      (e: any) => e.path
    );
    expect(paths, "criterion 5a: sobre.md listed").toContain("sobre.md");
    expect(paths, "criterion 5b: task listed").toContain(newTaskPath);
  });
});
