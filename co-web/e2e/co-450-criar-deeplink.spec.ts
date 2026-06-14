/**
 * CO-450 — Deep-link `?criar` handler (Yggdrasil "Criar no CO" round-trip)
 *
 * The Yggdrasil 🌐 button (YG-138) redirects an authenticated owner to
 *   /?criar=1&name=<nome>&key=<chave>&source=yggdrasil&instance=<id>
 * The CO side opens the CO-96 create-universe modal pre-filled and, once it
 * consumes the params, strips them from the URL so a refresh won't re-fire.
 *
 *  - Logged-in: modal opens prefilled with name/key; URL params are cleared.
 *  - Anonymous (expired session): login modal appears, prefill resumes after auth.
 *  - Invalid key: modal opens showing the CO-96 inline error (no blind create).
 */

import { test, expect } from "./fixtures";

const BASE = process.env.BASE_URL ?? "http://localhost:3000";

async function loginYuri(page: import("@playwright/test").Page) {
  // CO_ENV=test/uat enables uat-login; the POST sets the session cookie on the
  // page context so the following goto() boots authenticated.
  await page.request.post(`${BASE}/api/v1/auth/uat-login`, {
    data: { email: "yuri@uat.local", password: "uat" },
  });
}

test.describe("CO-450: ?criar deep-link", () => {
  test("logged-in: modal opens pre-filled and URL params are consumed", async ({ page }) => {
    await loginYuri(page);
    const key = `yg-deeplink-${Date.now()}`;
    await page.goto(`/?criar=1&name=Foo%20Bar&key=${key}&source=yggdrasil&instance=inst-42`, {
      waitUntil: "networkidle",
    });

    const overlay = page.locator("#criar-modal-overlay");
    await expect(overlay).not.toHaveClass(/hidden/);
    await expect(page.locator("#criar-name")).toHaveValue("Foo Bar");
    await expect(page.locator("#criar-slug")).toHaveValue(key);

    // Params consumed → not present after history.replaceState.
    const search = await page.evaluate(() => window.location.search);
    expect(search).not.toContain("criar=1");
    expect(search).not.toContain("source=yggdrasil");
  });

  test("logged-in: submit creates the universe and redirects to /co/<key>", async ({ page }) => {
    await loginYuri(page);
    const key = `yg-create-${Date.now()}`;
    await page.goto(`/?criar=1&name=YG%20Create&key=${key}&source=yggdrasil&instance=inst-7`, {
      waitUntil: "networkidle",
    });

    const submit = page.locator("#criar-submit");
    await expect(submit).toBeEnabled();
    await submit.click();

    // CO-96 behaviour: on success the SPA navigates to the new universe.
    await page.waitForFunction(
      (k) => window.location.pathname === `/${k}`,
      key,
      { timeout: 15000 },
    );
  });

  test("anonymous: deep-link shows the login flow (auth-aware), no create modal", async ({ page }) => {
    // No login → expired/absent session path.
    await page.context().clearCookies();
    await page.goto(`/?criar=1&name=Foo&key=foo-bar-anon&source=yggdrasil`, {
      waitUntil: "networkidle",
    });

    await expect(page.locator("#login-modal-overlay")).not.toHaveClass(/hidden/);
    await expect(page.locator("#criar-modal-overlay")).toHaveClass(/hidden/);
    // Params kept so the post-login reload can resume the prefill.
    const search = await page.evaluate(() => window.location.search);
    expect(search).toContain("criar=1");
  });

  test("invalid key: modal opens with the CO-96 inline error, submit disabled", async ({ page }) => {
    await loginYuri(page);
    await page.goto(`/?criar=1&name=Bad&key=X&source=yggdrasil`, {
      waitUntil: "networkidle",
    });

    await expect(page.locator("#criar-modal-overlay")).not.toHaveClass(/hidden/);
    // "X" is below the 2-char minimum and uppercase → CO-96 inline error fires.
    await expect(page.locator("#criar-error")).not.toHaveClass(/hidden/);
    await expect(page.locator("#criar-submit")).toBeDisabled();
  });
});
