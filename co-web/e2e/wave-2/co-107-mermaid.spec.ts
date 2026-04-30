import { test, expect } from "@playwright/test";

const BASE_URL = process.env.BASE_URL ?? "http://localhost:3000";

test.describe("CO-107: mermaid rendering", () => {
  test("template home renders mermaid svg", async ({ page }) => {
    await page.goto(`${BASE_URL}/co?u=template`);
    await page.waitForSelector(".universe-home-md", { timeout: 10000 });
    const svg = page.locator(".universe-home-md .co-mermaid svg");
    await expect(svg).toBeVisible({ timeout: 8000 });
    const text = await svg.textContent();
    expect(text).toContain("Tempo");
    expect(text).toContain("Universo");
    expect(text).toContain("Humanidade");
  });

  test(
    "universes without mermaid blocks do not load mermaid bundle",
    async ({ page }) => {
      const requests: string[] = [];
      page.on("request", (r) => {
        if (r.url().includes("mermaid")) requests.push(r.url());
      });
      await page.goto(`${BASE_URL}/co?u=quilomboaraucaria`);
      await page.waitForTimeout(2000);
      expect(requests).toEqual([]);
    },
  );
});
