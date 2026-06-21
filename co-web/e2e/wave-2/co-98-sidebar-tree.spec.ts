import { test, expect } from "@playwright/test";
import { loginAsAdmin } from "../helpers";

const BASE_URL = process.env.BASE_URL ?? "http://localhost:3000";

test.describe("CO-98: sidebar universe tree", () => {
  test(
    "logged-in user sees timeline trio nested under template in sidebar",
    async ({ page }) => {
      await loginAsAdmin(page);
      await page.goto(`${BASE_URL}/co?u=template`);
      await expect(page.locator(".sidebar-universes")).toBeVisible();
      // template parent is rendered with chevron
      await expect(
        page.locator('[data-universe="template"] .sidebar-universe-chevron'),
      ).toContainText(/[▾▸]/);
      // toggle collapse then re-expand
      await page.locator('[data-toggle="template"]').click();
      await page.locator('[data-toggle="template"]').click();
      // children appear with indent
      for (const child of ["tempo", "humanity", "universo"]) {
        const el = page.locator(`[data-universe="${child}"]`);
        await expect(el).toBeVisible();
        const padLeft = await el.evaluate((n) =>
          parseInt(getComputedStyle(n).paddingLeft, 10),
        );
        expect(padLeft).toBeGreaterThan(20);
      }
    },
  );
});

test.describe("CO-98: Explorar panel on template home", () => {
  test(
    "anonymous template home lists the three children as cards",
    async ({ page }) => {
      // renderUniverseHome only renders when no board project is selected.
      // Stub the template's projects to empty so the universe home (with the
      // Explorar panel) renders deterministically instead of the tutorial board.
      await page.route("**/api/v1/universes/template/projects", (route) =>
        route.fulfill({
          status: 200,
          contentType: "application/json",
          body: "[]",
        }),
      );
      await page.goto(`${BASE_URL}/co?u=template`);

      const explorar = page.locator(".universe-home-explorar");
      await expect(explorar).toBeVisible();
      // The timeline trio renders as cards linking to /<slug>.
      for (const slug of ["tempo", "universo", "humanity"]) {
        await expect(
          explorar.locator(`a.explorar-card[href="/${slug}"]`),
        ).toBeVisible();
      }
    },
  );
});
