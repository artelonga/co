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
