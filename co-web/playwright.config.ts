import { defineConfig, devices } from "@playwright/test";

const baseURL = process.env.BASE_URL ?? "http://localhost:3000";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 4 : undefined,
  testIgnore: ["**/archived/**", "**/wave-2/**", "**/interactions/**"],
  reporter: "html",
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  globalSetup: "./e2e/global-setup.ts",
  globalTeardown: "./e2e/global-teardown.ts",
  projects: [
    // Desktop viewports (1280×720)
    {
      name: "chromium-desktop",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 1280, height: 720 },
      },
    },
    {
      name: "firefox-desktop",
      use: {
        ...devices["Desktop Firefox"],
        viewport: { width: 1280, height: 720 },
      },
    },
    {
      name: "webkit-desktop",
      use: {
        ...devices["Desktop Safari"],
        viewport: { width: 1280, height: 720 },
      },
    },
    // Tablet viewports (768×1024)
    {
      name: "chromium-tablet",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 768, height: 1024 },
      },
    },
    {
      name: "firefox-tablet",
      use: {
        ...devices["Desktop Firefox"],
        viewport: { width: 768, height: 1024 },
      },
    },
    {
      name: "webkit-tablet",
      use: {
        ...devices["Desktop Safari"],
        viewport: { width: 768, height: 1024 },
      },
    },
    // Mobile viewports (375×812)
    {
      name: "chromium-mobile",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 375, height: 812 },
      },
    },
    {
      name: "firefox-mobile",
      use: {
        ...devices["Desktop Firefox"],
        viewport: { width: 375, height: 812 },
      },
    },
    {
      name: "webkit-mobile",
      use: {
        ...devices["Desktop Safari"],
        viewport: { width: 375, height: 812 },
      },
    },
  ],
});
