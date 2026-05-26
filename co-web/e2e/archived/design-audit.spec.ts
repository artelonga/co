/**
 * Design quality audit — captures screenshots of every palette × state
 * for critical visual review. Not a pass/fail test.
 */
import { test, expect } from "@playwright/test";
import { join } from "path";
import * as fs from "fs";

const BASE = "http://localhost:3000";
const OUT = join(__dirname, "../design-audit");

const PALETTES = [
  { key: "", label: "modern" },
  { key: "scholarly", label: "scholarly-light" },
  { key: "scholarly-dark", label: "scholarly-dark" },
  { key: "relic", label: "relic-dark" },
  { key: "relic-light", label: "relic-light" },
];

test.use({ viewport: { width: 1280, height: 800 } });

async function setPalette(page: any, key: string) {
  await page.evaluate((k: string) => {
    document.documentElement.setAttribute("data-palette", k);
    localStorage.setItem("co_named_palette", k);
  }, key);
  await page.waitForTimeout(300);
}

async function shot(page: any, name: string) {
  fs.mkdirSync(OUT, { recursive: true });
  await page.screenshot({
    path: join(OUT, `${name}.png`),
    fullPage: false,
  });
}

test.describe("Design audit", () => {
  // Seed a project + tasks so the board has content to render
  test.beforeAll(async ({ request }) => {
    // Create project if missing
    await request.post(`${BASE}/api/projects`, {
      data: { key: "ds", name: "Design System", description: "Design review" },
      headers: { "Content-Type": "application/json" },
    });
    const tasks = [
      { title: "Typography hierarchy review", status: "todo", priority: "high", labels: ["design"] },
      { title: "Color contrast audit", status: "in_progress", priority: "critical", labels: ["a11y"] },
      { title: "Component spacing pass", status: "in_review", priority: "medium", labels: ["layout"] },
      { title: "Dark mode validation", status: "done", priority: "low", labels: ["design"] },
    ];
    for (const task of tasks) {
      await request.post(`${BASE}/api/projects/ds/tasks`, {
        data: task,
        headers: { "Content-Type": "application/json" },
      });
    }
  });

  for (const palette of PALETTES) {
    test(`${palette.label}: login screen`, async ({ page }) => {
      await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
      await setPalette(page, palette.key);
      // Ensure login modal is showing
      await page.evaluate(() => {
        const overlay = document.getElementById("login-modal-overlay");
        if (overlay) overlay.classList.remove("hidden");
      });
      await page.waitForTimeout(400);
      await shot(page, `${palette.label}--login`);
    });

    test(`${palette.label}: kanban board`, async ({ page }) => {
      await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
      await setPalette(page, palette.key);
      // Hide login modal and navigate to board
      await page.evaluate(() => {
        const overlay = document.getElementById("login-modal-overlay");
        if (overlay) overlay.classList.add("hidden");
      });
      // Click ds project in sidebar (if present)
      const dsItem = page.locator(".sidebar-item[data-key='ds']");
      if (await dsItem.count() > 0) {
        await dsItem.click();
        await page.waitForTimeout(600);
      }
      await shot(page, `${palette.label}--kanban`);
    });

    test(`${palette.label}: task modal open`, async ({ page }) => {
      await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
      await setPalette(page, palette.key);
      await page.evaluate(() => {
        const overlay = document.getElementById("login-modal-overlay");
        if (overlay) overlay.classList.add("hidden");
        const modal = document.getElementById("modal-overlay");
        if (modal) modal.classList.remove("hidden");
      });
      await page.waitForTimeout(300);
      await shot(page, `${palette.label}--modal`);
    });

    test(`${palette.label}: palette switcher open`, async ({ page }) => {
      await page.goto(`${BASE}/`, { waitUntil: "networkidle" });
      await setPalette(page, palette.key);
      await page.evaluate(() => {
        const overlay = document.getElementById("login-modal-overlay");
        if (overlay) overlay.classList.add("hidden");
      });
      const toggle = page.locator("#palette-switcher-toggle");
      if (await toggle.count() > 0) {
        await toggle.click();
        await page.waitForTimeout(200);
      }
      await shot(page, `${palette.label}--switcher`);
    });
  }
});
