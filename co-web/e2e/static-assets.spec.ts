/**
 * CO-250 — Static-asset MIME type assertions
 *
 * For each critical asset URL, asserts:
 * - HTTP 200 status
 * - Content-Type header starts with the expected MIME prefix (case-insensitive)
 *
 * This spec would have caught the 2.13.3 regression where serve_deep_link
 * matched /{slug}/{*subpath} and served HTML instead of JS for paths like
 * /variants/a/app.js — causing browsers to refuse execution (MIME mismatch)
 * and leaving every user at a permanent "Carregando…" spinner.
 */

import { test, expect, request as playwrightRequest } from "@playwright/test";

interface AssetEntry {
  path: string;
  expectedMime: string;
}

const ASSETS: AssetEntry[] = [
  // SPA entry point and core modules
  { path: "/variants/a/app.js",             expectedMime: "application/javascript" },
  { path: "/variants/a/modules/api.js",     expectedMime: "application/javascript" },
  { path: "/variants/a/modules/sidebar.js", expectedMime: "application/javascript" },
  { path: "/variants/a/modules/boot.js",    expectedMime: "application/javascript" },
  { path: "/variants/a/style.css",          expectedMime: "text/css" },
  // Shared assets (served from /shared/ prefix)
  { path: "/shared/i18n.js",               expectedMime: "application/javascript" },
  { path: "/shared/markdown.js",           expectedMime: "application/javascript" },
  { path: "/shared/production.css",        expectedMime: "text/css" },
  { path: "/shared/experiment.css",        expectedMime: "text/css" },
  // PDF.js viewer bundle (served from /pdfjs/ prefix)
  { path: "/pdfjs/build/pdf.mjs",          expectedMime: "application/javascript" },
  // Root-level shortcuts (remapped server-side to shared/)
  // CO-357: PWA manifest uses the spec MIME type (W3C Web App Manifest)
  { path: "/manifest.json",                expectedMime: "application/manifest+json" },
  { path: "/sw.js",                        expectedMime: "application/javascript" },
];

test.describe("CO-250: static-asset MIME types", () => {
  for (const { path, expectedMime } of ASSETS) {
    test(`${path} → ${expectedMime}`, async () => {
      const base = process.env.BASE_URL ?? "http://localhost:3000";
      const ctx = await playwrightRequest.newContext({ baseURL: base });

      const resp = await ctx.get(path);

      expect(
        resp.status(),
        `expected HTTP 200 for ${path}`,
      ).toBe(200);

      const ct = (resp.headers()["content-type"] ?? "").toLowerCase();
      expect(
        ct,
        `${path}: content-type "${ct}" should start with "${expectedMime}"`,
      ).toContain(expectedMime.toLowerCase());

      await ctx.dispose();
    });
  }
});
