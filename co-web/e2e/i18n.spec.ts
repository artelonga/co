/**
 * CO-33 — i18n: language toggle pt↔en, UI strings update, cookie persistence
 *
 * Tests:
 *   - Toggle pt → en: all visible UI strings switch to English equivalents
 *   - Reload after toggle: language is restored from the co_lang cookie
 *   - Toggle en → pt: strings revert to Portuguese
 *   - Landing page (/co) language toggle works via the banner button
 */

import { test, expect } from "./fixtures";

// ─── Main app i18n toggle ─────────────────────────────────────────────────────

test.describe("i18n: language toggle on the main app", () => {
  test("language toggle button is present on the page", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    // The lang toggle appears in the login modal footer
    // Force modal open so the button is accessible
    await page.evaluate(() => {
      const el = document.getElementById("login-modal-overlay");
      if (el) el.classList.remove("hidden");
    });

    const langBtn = page.locator("#btn-lang-toggle");
    await expect(langBtn).toBeVisible();
  });

  test("clicking lang toggle from pt → en: button text switches to 'Português'", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    // Open login modal to access the lang toggle
    await page.evaluate(() => {
      const el = document.getElementById("login-modal-overlay");
      if (el) el.classList.remove("hidden");
    });

    const langBtn = page.locator("#btn-lang-toggle");
    await expect(langBtn).toBeVisible();

    // Default lang is pt — button shows "English"
    const initialText = await langBtn.textContent();
    expect(initialText?.trim()).toBe("English");

    // Toggle to English
    await langBtn.click();
    await page.waitForTimeout(300);

    // Button should now show "Português" (showing the other language)
    await expect(langBtn).toHaveText("Português");
  });

  test("toggling to English changes data-i18n text nodes in the page", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    await page.evaluate(() => {
      const el = document.getElementById("login-modal-overlay");
      if (el) el.classList.remove("hidden");
    });

    // Record Portuguese text for the login title
    const title = page.locator("#login-modal-title");
    const ptText = await title.textContent();
    expect(ptText?.trim()).toBe("Entrar");

    // Toggle to English
    await page.locator("#btn-lang-toggle").click();
    await page.waitForTimeout(300);

    // Title should now read "Sign In" (the en string for login_title)
    const enText = await title.textContent();
    expect(enText?.trim()).toBe("Sign In");
  });

  test("co_lang cookie is set after toggling language", async ({ page }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    await page.evaluate(() => {
      const el = document.getElementById("login-modal-overlay");
      if (el) el.classList.remove("hidden");
    });

    await page.locator("#btn-lang-toggle").click();
    await page.waitForTimeout(300);

    const cookies = await page.context().cookies();
    const langCookie = cookies.find((c) => c.name === "co_lang");
    expect(langCookie).toBeTruthy();
    expect(langCookie?.value).toBe("en");
  });

  test("co_lang cookie persists language across page reload", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    // Toggle to English
    await page.evaluate(() => {
      const el = document.getElementById("login-modal-overlay");
      if (el) el.classList.remove("hidden");
    });
    await page.locator("#btn-lang-toggle").click();
    await page.waitForTimeout(300);

    // Reload the page
    await page.reload({ waitUntil: "networkidle" });

    // Open modal again
    await page.evaluate(() => {
      const el = document.getElementById("login-modal-overlay");
      if (el) el.classList.remove("hidden");
    });
    await page.waitForTimeout(300);

    // Should still be in English
    const langBtn = page.locator("#btn-lang-toggle");
    await expect(langBtn).toHaveText("Português");

    const title = page.locator("#login-modal-title");
    await expect(title).toHaveText("Sign In");
  });

  test("toggling back to Portuguese after switching to English", async ({
    page,
  }) => {
    await page.goto("/", { waitUntil: "networkidle" });

    await page.evaluate(() => {
      const el = document.getElementById("login-modal-overlay");
      if (el) el.classList.remove("hidden");
    });

    const langBtn = page.locator("#btn-lang-toggle");

    // Switch to English
    await langBtn.click();
    await page.waitForTimeout(200);
    await expect(langBtn).toHaveText("Português");

    // Switch back to Portuguese
    await langBtn.click();
    await page.waitForTimeout(200);
    await expect(langBtn).toHaveText("English");

    // Title should be back to Portuguese
    const title = page.locator("#login-modal-title");
    await expect(title).toHaveText("Entrar");
  });
});

// ─── Landing page (/co) i18n ──────────────────────────────────────────────────

test.describe("i18n: language toggle on the /co landing page", () => {
  test("banner lang button is visible at /co", async ({ page }) => {
    await page.goto("/co", { waitUntil: "networkidle" });
    await expect(page.locator("#btn-banner-lang")).toBeVisible();
  });

  test("clicking banner lang toggle switches the hero title language", async ({
    page,
  }) => {
    await page.goto("/co", { waitUntil: "networkidle" });

    const heroTitle = page.locator("#template-hero-title");
    const ptText = await heroTitle.textContent();

    const langBtn = page.locator("#btn-banner-lang");
    await langBtn.click();
    await page.waitForTimeout(300);

    const enText = await heroTitle.textContent();
    expect(enText).not.toBe(ptText);
  });
});
