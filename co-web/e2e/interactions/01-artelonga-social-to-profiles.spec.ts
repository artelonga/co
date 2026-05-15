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
 *   5. A profile **stub** exists at
 *      `artelonga::comunidades/falcao.md` (`type: page`,
 *      `stub: true` in frontmatter) — the link target now resolves
 *      to a real entry, while the open task tracks the human work
 *      of filling it in.
 *   6. Both `sobre.md`, the new task, and the new stub are visible
 *      via the public entries API.
 *
 * SAFETY:
 *   - Original `sobre.md` body is snapshotted before mutation and
 *     restored in `afterEach`.
 *   - New task entry + falcao stub are deleted in `afterEach`.
 *   - If `CO_TEST_USER_EMAIL` / `CO_TEST_USER_PASSWORD` are not set,
 *     the test is skipped — a CI without secrets should not go red.
 *   - **Idempotent re-run**: if the precondition is already broken
 *     (e.g. a previous `afterEach` failed and `sobre.md` still has
 *     the post-state), the test skips with a clear message instead
 *     of overwriting state with garbage.
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
  const FALCAO_PROFILE_PATH = "comunidades/falcao.md";
  let falcaoStubCreated = false;

  test.beforeEach(async () => {
    ctx = await request.newContext({
      baseURL: BASE,
      ignoreHTTPSErrors: true,
    });
    await authenticate(ctx);
  });

  test.afterEach(async () => {
    const safeDelete = async (path: string) => {
      try {
        await ctx.delete(
          `/api/v1/universes/artelonga/entries/${path
            .split("/")
            .map(encodeURIComponent)
            .join("/")}`
        );
      } catch (_) {
        /* best-effort cleanup */
      }
    };

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
    if (newTaskPath) await safeDelete(newTaskPath);
    // Only delete the falcao stub if WE created it this run — never
    // wipe a stub a human/agent had legitimately filled out between
    // the spec creating it and afterEach running.
    if (falcaoStubCreated) await safeDelete(FALCAO_PROFILE_PATH);
    await ctx.dispose();
  });

  test("edit sobre.md + create falcao profile task + stub; all open", async () => {
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

    // Idempotent re-run guard: if the universe is already in the
    // post-state (no IG links remain), the previous afterEach probably
    // didn't fully restore. Skip with an explicit message instead of
    // writing nonsense in a body we don't recognise as the baseline.
    const hasIgLinks = originalBody.includes("https://www.instagram.com/");
    if (!hasIgLinks) {
      // Don't leave originalBody set — we'd "restore" garbage into prod.
      originalBody = "";
      test.skip(
        true,
        "Interaction state already mutated (no Instagram URLs found in sobre.md). " +
          "A prior run's afterEach did not restore the baseline. " +
          "Restore sobre.md manually (or via git history of the universe) before re-running."
      );
      return;
    }
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

    // AND: stub the falcao profile so [[falcao]] resolves to a real
    // page. The task created above tracks the human work of filling
    // out this stub; the stub itself flags `stub: true` so the
    // platform UI (or a future agent) can recognise it as
    // intentionally-incomplete.
    //
    // Only create the stub if it doesn't already exist — a real
    // profile someone wrote earlier must NOT be overwritten by a
    // test run.
    const existingStub = await ctx.get(
      `/api/v1/universes/artelonga/entries/${FALCAO_PROFILE_PATH
        .split("/")
        .map(encodeURIComponent)
        .join("/")}`
    );
    if (existingStub.status() === 404) {
      const stubFrontmatter = {
        type: "page",
        slug: "falcao",
        title: "Falcão",
        stub: true,
        tags: ["perfis", "comunidades", "stub:interaction-01"],
      };
      const stubBody = [
        "# Falcão",
        "",
        "*Esta página é um stub criado automaticamente pela INTERACTION-01.*",
        "",
        "**A completar** — ver task aberta em `projects/AL/" +
          String(nextId) +
          ".md`.",
        "",
        "Funções conhecidas:",
        "- skateboard",
        "- direção de ESG e transparência na ArteLonga",
        "",
      ].join("\n");
      const putStub = await ctx.put(
        `/api/v1/universes/artelonga/entries/${FALCAO_PROFILE_PATH
          .split("/")
          .map(encodeURIComponent)
          .join("/")}`,
        {
          headers: { "content-type": "application/json" },
          data: { body: stubBody, frontmatter: stubFrontmatter },
        }
      );
      expect(putStub.status(), "PUT falcao stub").toBeLessThan(400);
      falcaoStubCreated = true;
    }

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

    // 5. falcao profile stub exists, flagged as a stub
    const stubRes = await ctx.get(
      `/api/v1/universes/artelonga/entries/${FALCAO_PROFILE_PATH
        .split("/")
        .map(encodeURIComponent)
        .join("/")}`
    );
    expect(
      stubRes.status(),
      "criterion 5a: falcao stub GET 200"
    ).toBe(200);
    const stubEntry = await stubRes.json();
    expect(
      stubEntry.frontmatter?.stub === true ||
        String(stubEntry.body || "").toLowerCase().includes("stub"),
      "criterion 5b: stub flagged via frontmatter or body marker"
    ).toBe(true);

    // 6. All three entries appear in the public entries listing
    const list = await ctx.get(
      `/api/v1/universes/artelonga/entries?limit=500`
    );
    const listJson = await list.json();
    const paths: string[] = (listJson.entries ?? listJson ?? []).map(
      (e: any) => e.path
    );
    expect(paths, "criterion 6a: sobre.md listed").toContain("sobre.md");
    expect(paths, "criterion 6b: task listed").toContain(newTaskPath);
    expect(paths, "criterion 6c: falcao stub listed").toContain(
      FALCAO_PROFILE_PATH
    );
  });
});
