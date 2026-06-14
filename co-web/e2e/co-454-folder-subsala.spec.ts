/**
 * CO-454: Folder-as-sub-sala — a pasta node is descendable into a sub-sala whose
 * scope is that folder. Aligns CO's fractal layer with Yggdrasil `/mundo` rooms
 * (YG-146) so the map is 1:1: YG instance ↔ CO universe, YG room (pasta) ↔ CO
 * folder-sub-sala, YG nota ↔ CO nota.
 *
 * Tests:
 * 1. API: a folder-path slug (`default/jardim`, ridden as `default%2Fjardim`)
 *    round-trips through the workspace state API — a DISTINCT row from `default`,
 *    no new table (CO-352).
 * 2. Place a pasta → descend into its sub-sala → slug deepens (`default/jardim`),
 *    breadcrumb back-link shows → ascend → camera + parent scope restored.
 * 3. Root pasta `/` (no name) is inert on descend (it already IS this sala).
 */

import { test, expect } from "./fixtures";

test.describe("CO-454: folder-as-sub-sala descend/ascend", () => {
    // The sala canvas is a desktop-only surface (CO-352 / CO-359).
    test.beforeEach(async ({ page }) => {
        const vp = page.viewportSize();
        test.skip(!!(vp && vp.width <= 640), "Sala canvas is desktop-only (CO-352)");
    });

    test("API: folder-path slug round-trips as a distinct workspace state", async ({
        apiContext,
    }) => {
        const layout = {
            cells: {},
            notes: [],
            folders: [{ id: "root", name: "", x: 0, y: 0 }],
            universes: [],
            edges: [],
            view: { tx: 0, ty: 0, scale: 1 },
        };

        // The slug `default/jardim` rides the URL as one percent-encoded segment.
        const putRes = await apiContext.put(
            "/api/v1/universes/template/workspaces/default%2Fjardim/state",
            { data: { layout_json: JSON.stringify(layout), is_public: false } },
        );
        expect(putRes.status()).toBe(200);
        // The server decoded `%2F` back into the slug verbatim.
        expect((await putRes.json()).workspace_slug).toBe("default/jardim");

        const getRes = await apiContext.get(
            "/api/v1/universes/template/workspaces/default%2Fjardim/state",
        );
        expect(getRes.status()).toBe(200);
        const body = await getRes.json();
        expect(body.workspace_slug).toBe("default/jardim");
        const restored = JSON.parse(body.layout_json);
        expect(restored.view.scale).toBe(1);
    });

    test("place pasta → descend → ascend → camera + parent slug intact", async ({
        page,
    }) => {
        // Authenticate so saves persist (scheduleSave is a no-op for anon).
        await page.request.post("/api/v1/auth/uat-login", {
            data: { email: "yuri@uat.local", password: "uat" },
        });

        // Use a named parent slug (`co454`) so ascend reconstructs it verbatim —
        // the `default` slug is the URL's bare form (`/sala`), a separate path.
        await page.goto("/u/template/sala/co454", {
            waitUntil: "domcontentloaded",
        });
        await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });
        await page.waitForFunction(() => !!(window as any).__salaTest, null, {
            timeout: 10_000,
        });

        // Set a recognisable camera; parent sala is the `co454` workspace.
        await page.evaluate(() =>
            (window as any).__salaTest.setCam({ x: 480, y: 600, s: 1.4 }),
        );
        expect(
            await page.evaluate(() => (window as any).__salaTest.getSlug()),
        ).toBe("co454");

        // Place a pasta and descend into its sub-sala (deterministic — by id).
        await page.evaluate(() => {
            const t = (window as any).__salaTest;
            const id = t.addFolder("jardim", 3, 1);
            t.descendIntoFolder(id);
        });
        // The slug deepened to a folder path; URL carries it percent-encoded.
        await page.waitForURL("**/u/template/sala/co454%2Fjardim", {
            timeout: 10_000,
        });
        await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });
        await page.waitForFunction(() => !!(window as any).__salaTest, null, {
            timeout: 10_000,
        });

        // The sub-sala slug encodes the folder path under the parent (prefix).
        expect(
            await page.evaluate(() => (window as any).__salaTest.getSlug()),
        ).toBe("co454/jardim");

        // Breadcrumb back-link must appear (we descended from a parent sala).
        const back = page.locator("#sala-back");
        await expect(back).toBeVisible({ timeout: 5_000 });

        // Ascend via the breadcrumb → back to the parent slug.
        await back.click();
        await page.waitForURL("**/u/template/sala/co454", { timeout: 10_000 });
        await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });
        await page.waitForFunction(() => !!(window as any).__salaTest, null, {
            timeout: 10_000,
        });

        // Parent scope (slug) restored.
        expect(
            await page.evaluate(() => (window as any).__salaTest.getSlug()),
        ).toBe("co454");

        // Camera restored to where we left the parent sala.
        const cam = await page.evaluate(() =>
            (window as any).__salaTest.getCam(),
        );
        expect(Math.abs(cam.x - 480)).toBeLessThan(2);
        expect(Math.abs(cam.y - 600)).toBeLessThan(2);
        expect(Math.abs(cam.s - 1.4)).toBeLessThan(0.05);
    });

    test("root pasta `/` is inert on descend (it already is this sala)", async ({
        page,
    }) => {
        await page.request.post("/api/v1/auth/uat-login", {
            data: { email: "yuri@uat.local", password: "uat" },
        });
        await page.goto("/u/template/sala/default", {
            waitUntil: "domcontentloaded",
        });
        await expect(page.locator("#canvas-wrap")).toBeVisible({ timeout: 10_000 });
        await page.waitForFunction(() => !!(window as any).__salaTest, null, {
            timeout: 10_000,
        });

        // Descend into a nameless (root) pasta — must NOT navigate.
        const before = page.url();
        await page.evaluate(() => {
            const t = (window as any).__salaTest;
            const id = t.addFolder("", 1, 1); // nameless = root-like
            t.descendIntoFolder(id);
        });
        await page.waitForTimeout(300);
        expect(page.url()).toBe(before);
        await expect(page.locator("#canvas-wrap")).toBeVisible();
    });
});
