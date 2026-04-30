import { test, expect } from "@playwright/test";
import { loginAsAdmin } from "../helpers";

const BASE_URL = process.env.BASE_URL ?? "http://localhost:3000";

test.describe("CO-99: onboarding banner", () => {
  test(
    "first-time anonymous visit shows 3-step banner and dismisses",
    async ({ page, context }) => {
      await context.clearCookies();
      await page.setViewportSize({ width: 1024, height: 768 });
      await page.goto(`${BASE_URL}/co?u=template`);
      const banner = page.locator("#onboarding-banner");
      await expect(banner).toBeVisible({ timeout: 5000 });
      await expect(banner.locator("#onboarding-step-indicator")).toHaveText(
        "1 / 3",
      );
      await banner.locator("#onboarding-next").click();
      await expect(banner.locator("#onboarding-step-indicator")).toHaveText(
        "2 / 3",
      );
      await banner.locator("#onboarding-next").click();
      await expect(banner.locator("#onboarding-step-indicator")).toHaveText(
        "3 / 3",
      );
      await banner.locator("#onboarding-next").click();
      await expect(banner).toBeHidden();
      const cookies = await context.cookies();
      expect(cookies.find((c) => c.name === "co_onboarded")?.value).toBe("1");
      await page.reload();
      await expect(banner).toBeHidden();
    },
  );

  test(
    "mobile viewport (<720px) suppresses banner entirely",
    async ({ page, context }) => {
      await context.clearCookies();
      await page.setViewportSize({ width: 600, height: 800 });
      await page.goto(`${BASE_URL}/co?u=template`);
      await page.waitForTimeout(2000);
      await expect(page.locator("#onboarding-banner")).toBeHidden();
    },
  );

  test(
    "logged-in user does NOT see banner on template",
    async ({ page, context }) => {
      await context.clearCookies();
      await loginAsAdmin(page);
      await page.goto(`${BASE_URL}/co?u=template`);
      await page.waitForTimeout(2000);
      await expect(page.locator("#onboarding-banner")).toBeHidden();
    },
  );
});
