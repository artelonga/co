import { defineConfig, devices } from "@playwright/test";

const stagingURL =
  process.env.STAGING_URL ?? "https://staging.co.artelonga.com.br";

export default defineConfig({
  testDir: ".",
  // Single worker: live staging has shared state; parallel runs collide on fixtures
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 1,
  timeout: 60_000,
  reporter: [
    [
      "html",
      {
        outputFolder: `playwright-report/${process.env.GITHUB_SHA ?? "local"}`,
        open: "never",
      },
    ],
    ["list"],
  ],
  use: {
    baseURL: stagingURL,
    trace: "on-first-retry",
    actionTimeout: 30_000,
  },
  projects: [
    {
      name: "Pixel 7",
      use: { ...devices["Pixel 7"] },
    },
    {
      name: "iPhone 14",
      use: { ...devices["iPhone 14"] },
    },
    {
      name: "iPad Pro",
      use: { ...devices["iPad Pro 11"] },
    },
  ],
});
