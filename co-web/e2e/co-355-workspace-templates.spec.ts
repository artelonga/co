/**
 * CO-355: Workspace template registry — `_workspace.yaml` per universe seeds Sala layouts.
 *
 * Tests:
 * 1. GET /api/v1/universes/{key}/workspace-templates always returns "blank".
 * 2. GET /api/v1/universes/{key}/workspace-templates/blank returns the blank template.
 * 3. POST /from-template/blank creates a workspace_state with an empty layout.
 * 4. Sala tab is visible in the view tabs.
 * 5. Clicking the Sala tab renders the workspace view with the "Nova Sala" button.
 * 6. Clicking "Nova Sala" opens the template picker modal with at least "Blank".
 * 7. Selecting "Blank" creates a workspace state (API call succeeds + canvas renders).
 */

import { test, expect } from "./fixtures";
import { navigateTo } from "./helpers";

const UNIVERSE = "template";

test.describe("CO-355: workspace template registry", () => {
  // ─── API: template listing ──────────────────────────────────────────────────

  test("GET workspace-templates always includes blank", async ({ anonContext }) => {
    const res = await anonContext.get(
      `/api/v1/universes/${UNIVERSE}/workspace-templates`
    );
    expect(res.status()).toBe(200);
    const templates: Array<{ slug: string; name: string }> = await res.json();
    expect(Array.isArray(templates)).toBe(true);
    const slugs = templates.map((t) => t.slug);
    expect(slugs).toContain("blank");
  });

  test("GET workspace-templates/blank returns the blank template", async ({
    anonContext,
  }) => {
    const res = await anonContext.get(
      `/api/v1/universes/${UNIVERSE}/workspace-templates/blank`
    );
    expect(res.status()).toBe(200);
    const tpl: { slug: string; name: string } = await res.json();
    expect(tpl.slug).toBe("blank");
    expect(tpl.name).toBe("Blank");
  });

  test("GET workspace-templates/nonexistent returns 404", async ({
    anonContext,
  }) => {
    const res = await anonContext.get(
      `/api/v1/universes/${UNIVERSE}/workspace-templates/nonexistent-xyz`
    );
    expect(res.status()).toBe(404);
  });

  test("POST from-template/blank creates workspace state (anon)", async ({
    anonContext,
  }) => {
    const res = await anonContext.post(
      `/api/v1/universes/${UNIVERSE}/workspaces/test-sala-${Date.now()}/from-template/blank`,
      { data: {} }
    );
    expect(res.status()).toBe(201);
    const ws: {
      id: string;
      universe_key: string;
      template_slug: string;
      layout_json: string;
    } = await res.json();
    expect(ws.id).toBeTruthy();
    expect(ws.universe_key).toBe(UNIVERSE);
    expect(ws.template_slug).toBe("blank");
    const layout = JSON.parse(ws.layout_json);
    expect(Array.isArray(layout.nodes)).toBe(true);
    expect(layout.nodes.length).toBe(0);
  });

  test("POST from-template/nonexistent returns 404", async ({ anonContext }) => {
    const res = await anonContext.post(
      `/api/v1/universes/${UNIVERSE}/workspaces/test-sala/from-template/nonexistent-xyz`,
      { data: {} }
    );
    expect(res.status()).toBe(404);
  });

  // ─── UI: Sala tab + picker modal ───────────────────────────────────────────

  test("Sala tab is visible in view-tabs", async ({ page }) => {
    await navigateTo(page, "/co");
    await page.waitForSelector("#view-tabs", { timeout: 10_000 });
    const tab = page.locator('[data-view="workspace"]');
    await expect(tab).toBeVisible();
    await expect(tab).toContainText("Sala");
  });

  test("clicking Sala tab renders workspace view with Nova Sala button", async ({
    page,
  }) => {
    await navigateTo(page, "/co");
    await page.waitForSelector("#view-tabs", { timeout: 10_000 });

    await page.click('[data-view="workspace"]');
    await page.waitForSelector(".workspace-view", { timeout: 5_000 });

    const btn = page.locator("#ws-new-btn");
    await expect(btn).toBeVisible();
    await expect(btn).toContainText("Nova Sala");
  });

  test("clicking Nova Sala opens template picker with Blank option", async ({
    page,
  }) => {
    await navigateTo(page, "/co");
    await page.waitForSelector("#view-tabs", { timeout: 10_000 });

    await page.click('[data-view="workspace"]');
    await page.waitForSelector("#ws-new-btn", { timeout: 5_000 });
    await page.click("#ws-new-btn");

    await page.waitForSelector("#workspace-picker-overlay", { timeout: 5_000 });

    const blankItem = page.locator('.ws-template-item[data-slug="blank"]');
    await expect(blankItem).toBeVisible();
    await expect(blankItem).toContainText("Blank");
  });

  test("selecting Blank template creates workspace and opens the Sala surface", async ({
    page,
  }) => {
    await navigateTo(page, "/co");
    await page.waitForSelector("#view-tabs", { timeout: 10_000 });

    await page.click('[data-view="workspace"]');
    await page.waitForSelector("#ws-new-btn", { timeout: 5_000 });
    await page.click("#ws-new-btn");

    await page.waitForSelector(".ws-template-item[data-slug=\"blank\"]", {
      timeout: 5_000,
    });
    await page.click(".ws-template-item[data-slug=\"blank\"]");

    // The launcher hands off to the one Sala surface (CO-352,
    // docs/architecture/sala-surface.md) — never an in-SPA canvas.
    await page.waitForURL(/\/u\/[^/]+\/sala\/sala-\d+/, { timeout: 8_000 });
    await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });
  });
});
