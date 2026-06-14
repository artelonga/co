/**
 * CO-400: Universe-as-node — sala nodes can be universes; descend/ascend recursion.
 *
 * Tests:
 * 1. API: a layout carrying a universe node round-trips through the state API
 *    (no breaking change — universe nodes are opaque JSON inside layout_json).
 * 2. Place a universe node → descend into its sala → breadcrumb back-link shows
 *    → ascend → camera state restored and the universe node still present.
 * 3. Cycle tolerated: a sala holding its own universe node renders without
 *    recursion, and activating it is inert (stays put, no reload loop).
 */

import { test, expect } from "./fixtures";

test.describe("CO-400: universe-as-node descend/ascend", () => {
    // The sala canvas is a desktop-only surface (CO-352 / CO-359).
    test.beforeEach(async ({ page }) => {
        const vp = page.viewportSize();
        test.skip(!!(vp && vp.width <= 640), "Sala canvas is desktop-only (CO-352)");
    });

    test("API: universe node round-trips through the workspace state API", async ({
        apiContext,
    }) => {
        const layout = {
            cells: {},
            notes: [],
            folders: [],
            universes: [
                { id: "u1", kind: "universe", key: "mse", name: "MSE", x: 3, y: -2 },
            ],
            edges: [],
            view: { tx: 0, ty: 0, scale: 1 },
        };

        const putRes = await apiContext.put(
            "/api/v1/universes/template/workspaces/co400-roundtrip/state",
            { data: { layout_json: JSON.stringify(layout), is_public: false } },
        );
        expect(putRes.status()).toBe(200);

        const getRes = await apiContext.get(
            "/api/v1/universes/template/workspaces/co400-roundtrip/state",
        );
        expect(getRes.status()).toBe(200);
        const restored = JSON.parse((await getRes.json()).layout_json);
        expect(restored.universes).toHaveLength(1);
        expect(restored.universes[0].kind).toBe("universe");
        expect(restored.universes[0].key).toBe("mse");
        expect(restored.universes[0].x).toBe(3);
        expect(restored.universes[0].y).toBe(-2);
    });

    test("place universe node → descend → ascend → camera state intact", async ({
        page,
    }) => {
        // Authenticate the page so saves persist (scheduleSave is a no-op for anon).
        await page.request.post("/api/v1/auth/uat-login", {
            data: { email: "yuri@uat.local", password: "uat" },
        });

        await page.goto("/u/template/sala/co400", { waitUntil: "domcontentloaded" });
        await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });

        // Wait for the test hook (installed in boot()).
        await page.waitForFunction(() => !!(window as any).__salaTest, null, {
            timeout: 10_000,
        });

        // Set a recognisable camera and place a universe node pointing at a child.
        await page.evaluate(() => {
            const t = (window as any).__salaTest;
            t.setCam({ x: 512, y: 640, s: 1.5 });
            t.addUniverse("mse-child", "MSE", 12);
        });
        expect(
            await page.evaluate(() => (window as any).__salaTest.hasUniverse("mse-child")),
        ).toBe(true);

        // Descend into the child universe's sala.
        await page.evaluate(() => (window as any).__salaTest.descendInto("mse-child"));
        await page.waitForURL("**/u/mse-child/sala", { timeout: 10_000 });

        // Breadcrumb back-link must appear (we descended from a parent sala).
        const back = page.locator("#sala-back");
        await expect(back).toBeVisible({ timeout: 5_000 });

        // Ascend via the breadcrumb.
        await back.click();
        await page.waitForURL("**/u/template/sala/co400", { timeout: 10_000 });
        await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });
        await page.waitForFunction(() => !!(window as any).__salaTest, null, {
            timeout: 10_000,
        });

        // Camera state restored to where we left the parent sala.
        const cam = await page.evaluate(() => (window as any).__salaTest.getCam());
        expect(Math.abs(cam.x - 512)).toBeLessThan(2);
        expect(Math.abs(cam.y - 640)).toBeLessThan(2);
        expect(Math.abs(cam.s - 1.5)).toBeLessThan(0.05);

        // The placed universe node persisted across the descend/ascend round trip.
        expect(
            await page.evaluate(() => (window as any).__salaTest.hasUniverse("mse-child")),
        ).toBe(true);
    });

    test("cycle tolerated: a sala holding its own universe node is inert on activate", async ({
        page,
    }) => {
        await page.request.post("/api/v1/auth/uat-login", {
            data: { email: "yuri@uat.local", password: "uat" },
        });
        await page.goto("/u/template/sala/co400-cycle", {
            waitUntil: "domcontentloaded",
        });
        await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });
        await page.waitForFunction(() => !!(window as any).__salaTest, null, {
            timeout: 10_000,
        });

        // Place a universe node pointing at THIS universe (a cycle).
        await page.evaluate(() =>
            (window as any).__salaTest.addUniverse("template", "Template", 7),
        );

        // Activating the self-referential node must not navigate/reload — it is
        // inert (descend is navigation, never recursion).
        const before = page.url();
        await page.evaluate(() => (window as any).__salaTest.descendInto("template"));
        await page.waitForTimeout(300);
        expect(page.url()).toBe(before);
        // Canvas still alive.
        await expect(page.locator("#canvas-wrap")).toBeVisible();
    });
});
