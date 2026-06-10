/**
 * CO-352: Workspace ("Sala") primitive — spatial canvas view.
 *
 * Tests:
 * 1. Anonymous user visits /u/template/sala → canvas renders (empty or loaded).
 * 2. Anonymous user clicks btn-add → login CTA modal appears.
 * 3. GET /api/v1/universes/template/workspaces/default/state → 200 with layout_json.
 * 4. PUT /api/v1/universes/template/workspaces/default/state (authed) → 200.
 * 5. GET /api/v1/workspace-states/{token} (unknown token) → 404.
 */

import { test, expect } from "./fixtures";
import { navigateTo } from "./helpers";

test.describe("CO-352: Sala (workspace) primitive", () => {
    test("anonymous visitor to /u/template/sala sees the canvas page", async ({
        page,
    }) => {
        await page.goto("/u/template/sala", { waitUntil: "domcontentloaded" });

        // The sala page must load — verify the canvas wrapper is present.
        await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });

        // Toolbar must be rendered.
        await expect(page.locator("#toolbar")).toBeVisible({ timeout: 5_000 });
    });

    test("anonymous visitor clicks btn-add → login CTA modal appears", async ({
        page,
    }) => {
        await page.goto("/u/template/sala", { waitUntil: "domcontentloaded" });
        await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });

        // btn-add is visible but disabled for anon (readonly mode applies).
        // Click it anyway — the button may be disabled OR it may trigger the modal.
        const btn = page.locator("#btn-add");
        await expect(btn).toBeVisible({ timeout: 5_000 });

        // For anon the page applies readonly (btn disabled); the login CTA shows
        // only when the user tries to interact.  We force-click to simulate JS
        // dispatch and check that the CTA modal appears.
        await btn.click({ force: true });

        // Login CTA modal should appear (it is always present in DOM, hidden until triggered).
        await expect(page.locator("#login-cta-backdrop.show")).toBeVisible({ timeout: 3_000 });
    });

    test("API: GET workspace state returns layout_json for any caller", async ({
        anonContext,
    }) => {
        const res = await anonContext.get(
            "/api/v1/universes/template/workspaces/default/state",
        );
        expect(res.status()).toBe(200);
        const body = await res.json();
        // Even when no state is saved, the handler returns an empty-canvas placeholder.
        expect(body).toHaveProperty("layout_json");
        const layout = JSON.parse(body.layout_json);
        expect(layout).toHaveProperty("nodes");
        expect(layout).toHaveProperty("edges");
    });

    test("API: PUT workspace state persists and GET restores layout", async ({
        apiContext,
    }) => {
        const layout = {
            nodes: [{ entry_path: "projects/CO/1.md", x: 100, y: 200 }],
            edges: [],
            view: { tx: 0, ty: 0, scale: 1 },
        };

        // Save layout.
        const putRes = await apiContext.put(
            "/api/v1/universes/template/workspaces/default/state",
            { data: { layout_json: JSON.stringify(layout), is_public: false } },
        );
        expect(putRes.status()).toBe(200);
        const saved = await putRes.json();
        expect(saved).toHaveProperty("layout_json");

        // Restore layout.
        const getRes = await apiContext.get(
            "/api/v1/universes/template/workspaces/default/state",
        );
        expect(getRes.status()).toBe(200);
        const restored = await getRes.json();
        const restoredLayout = JSON.parse(restored.layout_json);
        expect(restoredLayout.nodes).toHaveLength(1);
        expect(restoredLayout.nodes[0].entry_path).toBe("projects/CO/1.md");
    });

    test("API: GET share-token with unknown token → 404", async ({
        anonContext,
    }) => {
        const res = await anonContext.get(
            "/api/v1/workspace-states/nonexistent-token-xyz",
        );
        expect(res.status()).toBe(404);
    });

    test("API: POST share token for saved state → returns token", async ({
        apiContext,
    }) => {
        // Ensure state exists first.
        const layout = {
            nodes: [{ entry_path: "projects/CO/share-test.md", x: 0, y: 0 }],
            edges: [],
            view: { tx: 0, ty: 0, scale: 1 },
        };
        await apiContext.put(
            "/api/v1/universes/template/workspaces/share-test/state",
            { data: { layout_json: JSON.stringify(layout), is_public: false } },
        );

        const shareRes = await apiContext.post(
            "/api/v1/universes/template/workspaces/share-test/state/share",
        );
        expect(shareRes.status()).toBe(200);
        const body = await shareRes.json();
        expect(body).toHaveProperty("share_token");
        expect(typeof body.share_token).toBe("string");
        expect(body.share_token.length).toBeGreaterThan(0);

        // Resolve the share token.
        const resolveRes = await apiContext.get(
            `/api/v1/workspace-states/${body.share_token}`,
        );
        expect(resolveRes.status()).toBe(200);
        const state = await resolveRes.json();
        expect(state.workspace_slug).toBe("share-test");
    });
});
