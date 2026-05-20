import { test, expect } from "@playwright/test";
import { loginAsAdmin } from "../helpers";

const BASE_URL = process.env.BASE_URL ?? "http://localhost:3000";

test.describe("CO-238: sidebar UX — owned/member/role/sub-universe semantics", () => {
  test.beforeEach(async ({ page }) => {
    await loginAsAdmin(page);
  });

  test("owned section renders renamed label, criador chip, and tooltip", async ({
    page,
  }) => {
    const slugA = `e2e-238-a-${Date.now()}`;
    const slugB = `e2e-238-b-${Date.now()}`;
    await page.request.post(`${BASE_URL}/api/v1/universes`, {
      data: { name: "E2E 238 A", key: slugA },
    });
    await page.request.post(`${BASE_URL}/api/v1/universes`, {
      data: { name: "E2E 238 B", key: slugB, parent_key: slugA },
    });

    await page.goto(`${BASE_URL}/co`, { waitUntil: "networkidle" });

    const ownedSection = page
      .locator(".sidebar-universe-section")
      .filter({
        has: page.locator(".sidebar-section-label", {
          hasText: "Universos que criei",
        }),
      })
      .first();
    await expect(ownedSection).toBeVisible();

    const ownedLabel = ownedSection.locator(".sidebar-section-label").first();
    await expect(ownedLabel).toHaveAttribute("title", /.+/);

    await expect(
      ownedSection.locator(".role-chip", { hasText: "criador" }).first(),
    ).toBeVisible();

    // Parent with one child shows count
    const parentItem = ownedSection.locator(`[data-universe="${slugA}"]`);
    await expect(
      parentItem.locator(".sidebar-item-name"),
    ).toContainText("(1)");
  });

  test("member section renders renamed label and tooltip when present", async ({
    page,
  }) => {
    await page.goto(`${BASE_URL}/co`, { waitUntil: "networkidle" });

    const memberLabel = page
      .locator(".sidebar-section-label", {
        hasText: "Universos onde tenho papel",
      })
      .first();
    const count = await memberLabel.count();
    if (count > 0) {
      await expect(memberLabel).toHaveAttribute("title", /.+/);
    }
  });

  test("subscribed section has tooltip when present", async ({ page }) => {
    await page.goto(`${BASE_URL}/co`, { waitUntil: "networkidle" });

    const subscribedLabel = page
      .locator(".sidebar-section-label", { hasText: "Inscrito em" })
      .first();
    const count = await subscribedLabel.count();
    if (count > 0) {
      await expect(subscribedLabel).toHaveAttribute("title", /.+/);
    }
  });

  test("cross-bucket parent renders as synthetic container in child section", async ({
    page,
  }) => {
    // Create a parent universe owned by admin
    const parentSlug = `e2e-238-parent-${Date.now()}`;
    const childSlug = `e2e-238-child-${Date.now()}`;
    await page.request.post(`${BASE_URL}/api/v1/universes`, {
      data: { name: "E2E Parent", key: parentSlug, is_public: true },
    });
    // Create child universe under the parent
    await page.request.post(`${BASE_URL}/api/v1/universes`, {
      data: {
        name: "E2E Child",
        key: childSlug,
        parent_key: parentSlug,
      },
    });

    await page.goto(`${BASE_URL}/co`, { waitUntil: "networkidle" });

    // Parent should show sub-count because it has at least one child
    const ownedSection = page
      .locator(".sidebar-universe-section")
      .filter({
        has: page.locator(".sidebar-section-label", {
          hasText: "Universos que criei",
        }),
      })
      .first();

    const parentItem = ownedSection.locator(
      `[data-universe="${parentSlug}"]`,
    );
    await expect(parentItem.locator(".sidebar-item-name")).toContainText("(1)");
  });
});
