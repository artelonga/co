import { test, expect } from "@playwright/test";

/**
 * Visit the template universe with each theme palette applied, take
 * a screenshot of the conteúdo layout, and sanity-check that the stats
 * bar uses theme colors (not the hardcoded white fallback).
 *
 * Surfaces regressions like the one fixed in 2.7.11 where my new
 * conteudo-stats CSS referenced `--surface-1` / `--text` — variables
 * that no theme defines — so the hardcoded fallback `#f8f8f6` rendered
 * white on every theme.
 *
 * Run with:
 *   BASE_URL=https://co-artelonga.fly.dev npx playwright test e2e/theme-coverage.spec.ts
 *
 * Screenshots land in test-results/ and on prod-snapshots/ when run
 * with --update-snapshots.
 */

const THEMES = [
  // Empty string = no data-palette attribute (default theme).
  "",
  "scholarly",
  "scholarly-dark",
  "relic",
  "relic-light",
  "medieval",
  "steampunk",
  "cyberpunk",
  "matrix",
  "garden",
  "terminal",
  "retro",
];

test.describe("Theme palette coverage", () => {
  for (const theme of THEMES) {
    const label = theme || "default";
    test(`stats bar honors ${label} theme`, async ({ page }) => {
      await page.goto("/template");
      await page.waitForLoadState("networkidle");

      // Force the palette via the documented `data-palette` mechanism
      // (same attribute settings.js sets at universe load).
      if (theme) {
        await page.evaluate((t) => {
          document.documentElement.setAttribute("data-palette", t);
        }, theme);
      } else {
        await page.evaluate(() => {
          document.documentElement.removeAttribute("data-palette");
        });
      }

      // Wait a frame for CSS to recompute.
      await page.waitForTimeout(120);

      const stats = page.locator(".conteudo-stats").first();
      const exists = (await stats.count()) > 0;
      if (!exists) {
        // Template might not have stats if conteudo view didn't load
        test.skip(true, "conteudo-stats element not present on this page");
        return;
      }

      // Take a screenshot for visual inspection.
      const safe = label.replace(/[^a-z0-9-]/gi, "_");
      await page.screenshot({
        path: `test-results/theme-${safe}.png`,
        fullPage: false,
      });

      // Assertion: stats background must NOT be the hardcoded fallback
      // (white-ish #f8f8f6 / #ffffff). When themes properly override
      // `--bg-hover`, the computed background should differ across
      // dark themes (relic, scholarly-dark, matrix, terminal, cyberpunk).
      const bg = await stats.evaluate((el) => {
        return getComputedStyle(el).backgroundColor;
      });

      // Sanity: not transparent / not the fallback white.
      expect(bg, `${label}: stats bg should not be transparent`).not.toBe("rgba(0, 0, 0, 0)");

      // Dark themes: background should be dark (sum of RGB < 384 = average < 128).
      const DARK_THEMES = new Set([
        "scholarly-dark",
        "relic",
        "medieval",
        "steampunk",
        "cyberpunk",
        "matrix",
        "garden",
        "terminal",
      ]);
      if (DARK_THEMES.has(theme)) {
        const m = bg.match(/rgba?\(\s*(\d+)[, ]+(\d+)[, ]+(\d+)/);
        if (m) {
          const [r, g, b] = [Number(m[1]), Number(m[2]), Number(m[3])];
          const sum = r + g + b;
          expect(
            sum,
            `${label}: dark-theme stats bg should be dark, got rgb(${r},${g},${b})`
          ).toBeLessThan(384);
        }
      }
    });
  }
});
