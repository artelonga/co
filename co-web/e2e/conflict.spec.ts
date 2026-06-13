/**
 * CO-128 — Apple-style 4-way conflict UI.
 *
 * End-to-end proof of the conflict TRIGGER: two sessions open the same entry,
 * both edit it, and saving B then A surfaces a `409 Conflict` whose payload
 * carries BOTH versions — the data that drives the conflict-resolution modal.
 *
 * The modal's DOM/keyboard/apply-to-all behaviour is covered by the fast
 * component suite (components/__tests__/conflict-modal.test.ts); here we also
 * load the real modal module in a page and assert it renders both versions.
 */

import { test, expect } from "./fixtures";

const UNIVERSE = "e2e-test";

async function loginContext(playwright: any, baseURL: string) {
  const ctx = await playwright.request.newContext({ baseURL });
  const res = await ctx.post("/api/v1/auth/uat-login", {
    data: { email: "yuri@uat.local", password: "uat" },
  });
  if (!res.ok()) throw new Error(`uat-login failed (${res.status()})`);
  return ctx;
}

test.describe("CO-128: divergent edits surface a 409 with both versions", () => {
  test("save B then A → 409 carrying local + remote bodies", async ({
    apiContext,
    playwright,
  }) => {
    const baseURL = process.env.BASE_URL ?? "http://localhost:3000";
    const path = `notes/conflict-${Date.now()}.md`;

    // Both sessions open the entry at the same base revision.
    const created = await apiContext.post(
      `/api/v1/universes/${UNIVERSE}/entries`,
      {
        data: {
          path,
          frontmatter: { type: "note", title: "Conflict subject" },
          body: "Shared starting point.",
        },
      },
    );
    expect(created.ok()).toBeTruthy();
    const baseHash: string = (await created.json()).body_hash;
    expect(baseHash).toBeTruthy();

    // Session B saves first — the entry diverges from the shared base.
    const sessionB = await loginContext(playwright, baseURL);
    const resB = await sessionB.put(
      `/api/v1/universes/${UNIVERSE}/entries/${path}`,
      { data: { body: "Edited by device B.", base_hash: baseHash } },
    );
    expect(resB.status()).toBe(200);

    // Session A saves second on the now-stale base — must conflict.
    const sessionA = await loginContext(playwright, baseURL);
    const resA = await sessionA.put(
      `/api/v1/universes/${UNIVERSE}/entries/${path}`,
      { data: { body: "Edited by device A.", base_hash: baseHash } },
    );
    expect(resA.status()).toBe(409);

    const payload = await resA.json();
    expect(payload.error).toBe("conflict");
    expect(payload.conflict.path).toBe(path);
    expect(payload.conflict.kind).toBe("both_modified");
    // Both versions present: local = A's attempt, remote = B's stored copy.
    expect(payload.conflict.local.body).toBe("Edited by device A.");
    expect(payload.conflict.remote.body).toBe("Edited by device B.");
    expect(payload.conflict.base.body_hash).toBe(baseHash);

    await sessionB.dispose();
    await sessionA.dispose();
  });

  test("the modal module renders both versions in the page", async ({ page }) => {
    await page.goto("/");
    // Load the real SPA module and render a payload (no server round-trip).
    const text = await page.evaluate(async () => {
      const mod = await import("/modules/sync/conflict-modal.js");
      const { overlay } = mod.buildConflictModal(
        {
          universe_key: "u1",
          path: "notes/demo.md",
          kind: "both_modified",
          local: { body: "alpha\nLOCAL line\nomega", body_hash: "a" },
          remote: { body: "alpha\nREMOTE line\nomega", body_hash: "b" },
          base: { body_hash: "base" },
        },
        {},
      );
      document.body.appendChild(overlay);
      return overlay.textContent || "";
    });
    expect(text).toContain("LOCAL line");
    expect(text).toContain("REMOTE line");
    expect(text).toContain("notes/demo.md");
  });
});
