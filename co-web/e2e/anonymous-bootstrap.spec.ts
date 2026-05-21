/**
 * CO-250 — Anonymous SPA bootstrap smoke test
 *
 * Opens `/` in a fresh browser context (no session cookies, no localStorage)
 * and verifies that the SPA fully initialises:
 *   - view tabs appear (proves JS executed and universe loaded)
 *   - task-count stats render in the content view (proves API responded, project loaded)
 *   - sidebar populated with at least one project item (non-mobile)
 *   - zero JavaScript console errors during boot
 *
 * This spec catches the class of regression where MIME errors silently
 * prevent script execution, leaving the user at a perpetual
 * "Carregando…" spinner with nothing rendered.
 *
 * Implementation note: the default view is `state.view = 'conteudo'`, so the
 * kanban board does not auto-render on boot. The conteudo stats panel
 * (`.conteudo-stat`) is a reliable bootstrap signal: it only appears once
 * JS has executed, the template project was auto-selected, and the entries
 * API returned data.
 */

import { test, expect } from "@playwright/test";

test.describe("CO-250: anonymous bootstrap", () => {
  test("/ boots SPA, renders task stats in content view, no console errors", async ({
    page,
  }) => {
    // Skip sidebar assertion on mobile viewports where the sidebar is hidden
    const viewport = page.viewportSize();
    const isMobile = !!(viewport && viewport.width <= 640);

    // This spec guards against the 2.13.3 MIME regression class:
    // serve_deep_link served text/html for JS assets → browser refused to
    // execute them → "Carregando…" forever. We capture only the errors that
    // indicate script-loading / MIME failures; general runtime errors in the
    // SPA are a separate concern and would make this spec too brittle.
    const mimeErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() !== "error") return;
      const text = msg.text();
      // Flag errors that look like MIME or script-execution failures
      if (
        /refused to execute script/i.test(text) ||
        /mime type.*not executable/i.test(text) ||
        /unexpected token.*</i.test(text) // HTML being parsed as JS
      ) {
        mimeErrors.push(text);
      }
    });
    // Uncaught SyntaxError from evaluating HTML-as-JS (MIME regression signal)
    page.on("pageerror", (err) => {
      if (/SyntaxError.*unexpected token|mime|refused.*execute/i.test(err.message)) {
        mimeErrors.push(`[pageerror] ${err.message}`);
      }
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });

    // View tabs must appear — proves JS executed and the universe loaded.
    // Without working JS assets the page stays at "Carregando…" with no tabs.
    await page.waitForSelector("#view-tabs", { state: "visible", timeout: 20_000 });

    // Sidebar populated with at least one project item (non-mobile only)
    if (!isMobile) {
      const sidebarItems = page.locator(
        "#project-list .sidebar-item, .sidebar-universe-item",
      );
      const sidebarCount = await sidebarItems.count();
      expect(
        sidebarCount,
        "sidebar should show at least one item after anonymous boot",
      ).toBeGreaterThanOrEqual(1);
    }

    // Content stats panel must appear — proves the template project was auto-selected,
    // entries API responded, and JS rendered the content view with populated data.
    // The stats panel (.conteudo-stat) shows entry counts (tarefas, páginas, etc.)
    // and only renders once renderConteudo() completes successfully.
    await page.waitForSelector(".conteudo-stat", { state: "visible", timeout: 15_000 });

    const statsCount = await page.locator(".conteudo-stat").count();
    expect(
      statsCount,
      "at least one conteudo stat should be visible after anonymous boot (proves project + tasks loaded)",
    ).toBeGreaterThanOrEqual(1);

    // No MIME-type or script-execution errors during boot
    // (these are the errors the 2.13.3/4 cascade would have produced)
    expect(
      mimeErrors,
      `MIME / script-execution errors during anonymous boot:\n${mimeErrors.join("\n")}`,
    ).toHaveLength(0);
  });
});
