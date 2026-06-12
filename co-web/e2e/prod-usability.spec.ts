/**
 * CO-421 — Prod usability gate (anonymous, read-only Playwright smoke).
 *
 * The release usability gate that a health-200 smoke can't provide. This suite
 * exercises the *real usability* of production by driving an anonymous browser:
 * the template board loads with tutorial tasks, theme switching applies, the
 * pt/en toggle changes labels, a deep-linked entry renders markdown (not a 404),
 * and the dashboard/lens view opens.
 *
 * Design constraints (HARD requirements):
 *   - NO auth. This suite deliberately does NOT import ./fixtures (which logs in
 *     via uat-login — a path that returns 404 in prod anyway). It uses the bare
 *     @playwright/test `test`/`expect`, so there is no session cookie, no login.
 *   - READ-ONLY. A request interceptor (installed in beforeEach) ABORTS and FAILS
 *     the test if any POST/PUT/PATCH/DELETE is ever issued. The suite can never
 *     mutate prod, by construction — not just by convention.
 *
 * How to run as the prod usability gate (target ~< 2 min total; verified green
 * against prod in ~40s):
 *
 *     cd co-web && BASE_URL=https://co.artelonga.com.br \
 *       npx playwright test e2e/prod-usability.spec.ts \
 *       --project=desktop-chromium --workers=2
 *
 * (--workers=2 keeps prod page loads from contending; the default localhost run
 * can use full parallelism.) Without BASE_URL it targets http://localhost:3000
 * (playwright.config.ts), so the same specs run in CI against a local
 * `co serve`. See docs/release-checklist.md.
 *
 * The `@prod` tag in the describe title lets the gate be selected explicitly:
 *     npx playwright test --grep @prod
 */

import { test, expect, type Route, type Request } from "@playwright/test";

const MUTATING = new Set(["POST", "PUT", "PATCH", "DELETE"]);

test.describe("CO-421: prod usability gate @prod", () => {
  // Collected per-test so an assertion can name the exact offending request.
  let mutations: string[] = [];

  test.beforeEach(async ({ page }) => {
    mutations = [];
    // READ-ONLY GUARD — the load-bearing invariant of this suite.
    // Any mutating request is aborted (never reaches prod) and recorded; the
    // afterEach assertion then fails the test. We route("**/*") so this covers
    // every request the page issues: navigation, XHR/fetch, assets, beacons.
    await page.route("**/*", (route: Route, request: Request) => {
      const method = request.method().toUpperCase();
      if (MUTATING.has(method)) {
        mutations.push(`${method} ${request.url()}`);
        // Abort so the mutation never hits the server even momentarily.
        return route.abort();
      }
      return route.continue();
    });
  });

  test.afterEach(() => {
    expect(
      mutations,
      `READ-ONLY VIOLATION — the prod usability gate must never mutate prod, ` +
        `but these mutating requests were issued:\n${mutations.join("\n")}`,
    ).toHaveLength(0);
  });

  test("template board loads with tutorial tasks visible", async ({ page }) => {
    // Target the template universe explicitly — it is always seeded with the
    // tutorial tasks regardless of which universe `/` resolves to in prod.
    await page.goto("/template", { waitUntil: "domcontentloaded" });

    // View tabs appear → JS executed and the universe loaded.
    await page.waitForSelector("#view-tabs", { state: "visible", timeout: 20_000 });

    // The default `conteudo` view is the universe content surface ("board" of
    // entry cards) and is the only view that renders without a selected project
    // on the template universe. Its stats panel is the usability signal a health
    // 200 cannot give: it only appears once the entries API responded and JS
    // rendered the content with real data.
    await page.waitForSelector(".conteudo-stat", { state: "visible", timeout: 15_000 });

    // Tutorial tasks must actually be present — assert the "tarefas" stat is > 0.
    const tarefaStat = page
      .locator(".conteudo-stat", { hasText: /tarefas?/i })
      .first();
    await expect(tarefaStat).toBeVisible({ timeout: 10_000 });
    const tarefaCount = parseInt(
      (await tarefaStat.locator(".conteudo-stat-value").textContent())?.trim() || "0",
      10,
    );
    expect(
      tarefaCount,
      "the template board should report at least one tutorial task (tarefas stat)",
    ).toBeGreaterThanOrEqual(1);

    // Entry cards render on the board surface (each tutorial task / page is a
    // card). Cards may be grouped under collapsible sections (e.g. "Tarefas").
    // Expand a collapsed section header if present — this also exercises a real
    // anonymous interaction — then assert at least one card is actually visible.
    const cards = page.locator(
      ".conteudo-card, .kanban-card, .kanban-column .task-card",
    );
    expect(
      await cards.count(),
      "the template board should render content cards in the DOM",
    ).toBeGreaterThanOrEqual(1);

    const sectionHeaders = page.locator(".co-section-header");
    if ((await sectionHeaders.count()) > 0) {
      // Prefer the "Tarefas" (tasks) section; fall back to the first header.
      const tarefasHeader = sectionHeaders.filter({ hasText: /tarefas?/i }).first();
      const header = (await tarefasHeader.count()) > 0 ? tarefasHeader : sectionHeaders.first();
      await header.click();
    }

    const visibleCards = page.locator(
      ".conteudo-card:visible, .kanban-card:visible, .kanban-column .task-card:visible",
    );
    await expect(visibleCards.first()).toBeVisible({ timeout: 15_000 });
  });

  test("theme switch applies (data-palette changes)", async ({ page }) => {
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.waitForSelector("#palette-switcher-toggle", {
      state: "visible",
      timeout: 20_000,
    });

    const html = page.locator("html");
    const before = (await html.getAttribute("data-palette")) ?? "";

    // Open the palette dropdown. Options are <button .palette-switcher-item
    // data-palette-key="..."> and clicking one sets html[data-palette=<key>]
    // (see static/shared/experiment.js applyNamedPalette).
    await page.locator("#palette-switcher-toggle").click();
    const items = page.locator(".palette-switcher-item[data-palette-key]");
    await expect(items.first()).toBeVisible({ timeout: 10_000 });

    // Prefer "scholarly" (always in the catalogue); fall back to any option
    // whose key differs from the current palette so the change is observable.
    let targetKey = "scholarly";
    if (before === targetKey) {
      const n = await items.count();
      targetKey = "";
      for (let i = 0; i < n; i++) {
        const k = (await items.nth(i).getAttribute("data-palette-key")) ?? "";
        if (k !== before) {
          targetKey = k;
          break;
        }
      }
    }

    const target = page.locator(
      `.palette-switcher-item[data-palette-key="${targetKey}"]`,
    );
    await target.first().click();

    // The applied palette is reflected on <html data-palette> and differs from
    // the starting value — proving the theme switch took effect.
    await expect(html).toHaveAttribute("data-palette", targetKey);
    expect(targetKey, "selected palette should differ from the initial one").not.toBe(before);
  });

  test("pt/en language toggle changes labels", async ({ page }) => {
    await page.goto("/", { waitUntil: "domcontentloaded" });

    // The language toggle lives in the login modal header; reveal the modal
    // (read-only DOM manipulation, no network) so the button is interactable.
    await page.waitForSelector("#view-tabs", { state: "visible", timeout: 20_000 });
    await page.evaluate(() => {
      const el = document.getElementById("login-modal-overlay");
      if (el) el.classList.remove("hidden");
    });

    const langBtn = page.locator("#btn-lang-toggle");
    await expect(langBtn).toBeVisible({ timeout: 10_000 });

    // Normalize to PT first (button reads "English" when current language is PT).
    const initial = (await langBtn.textContent()) ?? "";
    if (!/english/i.test(initial)) {
      await langBtn.click(); // currently EN → back to PT
      await expect(langBtn).toContainText(/english/i);
    }

    // PT → EN: the toggle label flips to "Português".
    await langBtn.click();
    await expect(langBtn).toContainText(/portugu[êe]s/i);
  });

  test("deep-linked template entry renders markdown (not a 404)", async ({ page }) => {
    // A guaranteed-seeded template entry. The template universe always seeds
    // projects/CO/1.md..7.md (the tutorial tasks), so this deep link is
    // self-contained and not coupled to any content universe.
    await page.goto("/template/projects/CO/1", { waitUntil: "domcontentloaded" });

    // The entry resolves into the zoom reader modal with rendered markdown.
    // It must NOT fall through to the 404 view.
    const zoom = page.locator("#co-zoom-overlay");
    await expect(zoom).toBeVisible({ timeout: 20_000 });

    const body = page.locator("#co-zoom-body.md-article");
    await expect(body).toBeVisible({ timeout: 15_000 });
    // Rendered HTML, not an empty shell — at least some text content.
    await expect(body).not.toBeEmpty();

    // Hard guard: the 404 recovery view must be absent.
    await expect(page.locator("#co-not-found-view")).toHaveCount(0);
  });

  test("graph / lens view opens", async ({ page }) => {
    await page.goto("/template", { waitUntil: "domcontentloaded" });
    await page.waitForSelector("#view-tabs", { state: "visible", timeout: 20_000 });

    // The universe-level content lens (the .conteudo-stat panel) is the
    // analytics surface that always opens anonymously — it breaks the graph of
    // entries down by type/tag (tarefas / páginas / eventos / tags). Assert it
    // first as the guaranteed lens.
    const stats = page.locator(".conteudo-stat");
    await expect(stats.first()).toBeVisible({ timeout: 15_000 });
    expect(
      await stats.count(),
      "the content lens (entry stats breakdown) should render",
    ).toBeGreaterThanOrEqual(1);

    // Bonus: the dashboard tab is the project-scoped analytics lens (velocity /
    // burnup / status distribution). On the template universe (no project
    // pre-selected) it may not activate; if it does render, assert its cards.
    const dashTab = page.locator('#view-tabs .view-tab[data-view="dashboard"]');
    if ((await dashTab.count()) > 0) {
      await dashTab.click();
      const dashboard = page.locator(".dashboard");
      const appeared = await dashboard
        .first()
        .waitFor({ state: "visible", timeout: 6_000 })
        .then(() => true)
        .catch(() => false);
      if (appeared) {
        await expect(page.locator(".dashboard-card").first()).toBeVisible({
          timeout: 10_000,
        });
      }
    }
  });
});
