/**
 * CO-356 — Board drag: pointer-event based DnD (replaces HTML5 DnD)
 *
 * Covers:
 *   - Card → column drag on desktop (mouse)
 *   - Mobile viewport drag: iPhone 13 (390×844), iPad Mini (768×1024), Pixel 5 (393×851)
 *   - Threshold guards: quick tap and vertical swipe do NOT trigger drag
 *   - Full CRUD sequence: create → drag → edit → delete
 */

import { test, expect } from "./fixtures";
import {
  navigateTo,
  selectProject,
  waitForBoard,
  createTask,
} from "./helpers";
import type { Locator, Page } from "@playwright/test";

// ─── Helpers ─────────────────────────────────────────────────────────────────

/**
 * Open the mobile hamburger sidebar, select a project, then close the sidebar
 * so the content area is accessible for interaction.
 * Used on viewports ≤ 640 px where the sidebar is hidden behind a hamburger menu.
 */
async function selectProjectOnMobile(page: Page, key: string): Promise<void> {
  await page.locator("#hamburger-btn").click();
  // Wait for sidebar to slide open (CSS class toggles display)
  await page.locator("#sidebar.open").waitFor({ state: "attached", timeout: 5_000 });

  const tasksLoaded = page.waitForResponse(
    (r) =>
      r.url().includes(`/api/projects/${key}/tasks`) && r.status() === 200,
    { timeout: 15_000 },
  );
  await page.locator(`#project-list .sidebar-item-key:text-is("${key}")`).click();
  await tasksLoaded;

  // Close sidebar overlay so view tabs are reachable. The open sidebar
  // covers the overlay's center point and intercepts a normal click, so
  // dispatch the click event straight to the overlay element.
  await page.locator("#sidebar-overlay.visible").dispatchEvent("click");
  await page.waitForFunction(
    () => !document.querySelector("#sidebar.open"),
    { timeout: 5_000 },
  );

  await page
    .waitForFunction(
      () => !document.querySelector("#content .loading-spinner"),
      { timeout: 10_000 },
    )
    .catch(() => {});

  await page.locator('#view-tabs .view-tab[data-view="kanban"]').click();
  await page.waitForSelector(".kanban-column", {
    state: "visible",
    timeout: 10_000,
  });
}

/**
 * Simulate a long-press drag: hold > 200 ms (activating the hold threshold in
 * pointer-drag.js) then move to the target element.  Used on mobile viewports
 * where kanban columns are stacked vertically so the drag is downward, not
 * horizontal, and the 8 px horizontal threshold never triggers.
 *
 * The 250 ms wait is intentional — it tests the 200 ms hold threshold feature
 * in pointer-drag.js, not an arbitrary sleep.
 */
async function holdAndDrag(page: Page, from: Locator, to: Locator): Promise<void> {
  const fromBox = await from.boundingBox();
  const toBox = await to.boundingBox();
  if (!fromBox || !toBox) throw new Error("holdAndDrag: bounding boxes null");

  const fx = fromBox.x + fromBox.width / 2;
  const fy = fromBox.y + fromBox.height / 2;
  const tx = toBox.x + toBox.width / 2;
  const ty = toBox.y + toBox.height / 2;

  await page.mouse.move(fx, fy);
  await page.mouse.down();
  await page.waitForTimeout(250); // exceed 200 ms hold threshold
  await page.mouse.move(tx, ty, { steps: 10 });
  await page.mouse.up();
}

// ─── Desktop drag ─────────────────────────────────────────────────────────────
// ─── Drag between columns ─────────────────────────────────────────────────────

test.describe("Drag: move task card between kanban columns", () => {
  // ≤640px shows a single column (CO-358) — cross-column drag at that width
  // is covered by the dedicated "Mobile drag" suite below via segment buttons.
  test.beforeEach(async ({ page }) => {
    test.skip((page.viewportSize()?.width ?? 1280) <= 640,
      "single-column mobile board — covered by the Mobile drag suite");
  });

  test("drag from 'To Do' to 'In Progress' updates the task status via API", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Drag test task",
      status: "todo",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    await expect(card).toBeVisible();

    const inProgressCol = page
      .locator('.kanban-column[data-status="in_progress"]')
      .or(page.locator(".kanban-column").filter({ hasText: "In Progress" }));
    await expect(inProgressCol).toBeVisible();

    await card.dragTo(inProgressCol);

    await expect(
      inProgressCol.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible({ timeout: 5_000 });
  });

  test("drag from 'In Progress' to 'Done' column", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Done drag task",
      status: "in_progress",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    await expect(card).toBeVisible();

    const doneCol = page.locator('.kanban-column[data-status="done"]');

    await card.dragTo(doneCol);

    await expect(
      doneCol.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible({ timeout: 5_000 });
  });
});

// ─── Drag threshold guards ─────────────────────────────────────────────────────

test.describe("Drag threshold: tap and vertical swipe do not trigger drag", () => {
  test("quick tap (< 8 px movement) does not move the card", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Tap guard task",
      status: "todo",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    const box = await card.boundingBox();
    if (!box) throw new Error("card bounding box null");

    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;

    // Press, move only 4 px horizontally (below 8 px threshold), release.
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx + 4, cy);
    await page.mouse.up();

    // Card must remain in the todo column — no drag fired.
    const todoCol = page.locator('.kanban-column[data-status="todo"]');
    await expect(
      todoCol.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible();
  });

  test("vertical swipe on a card does not trigger drag", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Scroll guard task",
      status: "todo",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    const box = await card.boundingBox();
    if (!box) throw new Error("card bounding box null");

    const cx = box.x + box.width / 2;
    const cy = box.y + box.height / 2;

    // Purely vertical swipe (dy=40 >> dx=0) — should cancel gesture as scroll.
    await page.mouse.move(cx, cy);
    await page.mouse.down();
    await page.mouse.move(cx, cy - 40, { steps: 5 });
    await page.mouse.up();

    const todoCol = page.locator('.kanban-column[data-status="todo"]');
    await expect(
      todoCol.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible();
  });
});

// ─── Mobile viewport drag ─────────────────────────────────────────────────────

const MOBILE_VIEWPORTS = [
  { name: "iPhone 13", width: 390, height: 844 },
  { name: "iPad Mini", width: 768, height: 1024 },
  { name: "Pixel 5", width: 393, height: 851 },
] as const;

for (const device of MOBILE_VIEWPORTS) {
  test.describe(`Mobile drag — ${device.name} (${device.width}×${device.height})`, () => {
    test.use({ viewport: { width: device.width, height: device.height } });

    test(`drag card from 'To Do' to 'In Progress' on ${device.name}`, async ({
      page,
      apiContext,
      seedProject,
    }) => {
      const task = await createTask(apiContext, seedProject.key, {
        title: `Mobile drag — ${device.name}`,
        status: "todo",
      });

      await navigateTo(page, "/");

      // Sidebar is hidden at ≤ 640 px (single-column kanban layout); open it via
      // the hamburger button.  At 768 px (iPad Mini) the sidebar is always visible.
      if (device.width <= 640) {
        await selectProjectOnMobile(page, seedProject.key);
      } else {
        await selectProject(page, seedProject.key);
      }

      const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
      await expect(card).toBeVisible();

      const inProgressCol = page.locator('.kanban-column[data-status="in_progress"]');

      // CO-358: at ≤ 640 px the board shows ONE column (segmented control on
      // top); the other columns are hidden, so the cross-column drop target
      // is the segment button. The view then follows the card to its new
      // column. On 768 px (iPad Mini) columns are side by side — dragTo()
      // onto the visible column works directly.
      if (device.width <= 640) {
        const segBtn = page.locator('.kanban-segment-btn[data-status="in_progress"]');
        await expect(segBtn).toBeVisible();
        await holdAndDrag(page, card, segBtn);
      } else {
        await expect(inProgressCol).toBeVisible();
        await card.dragTo(inProgressCol);
      }

      await expect(
        inProgressCol.locator(`.task-card[data-task-id="${task.id}"]`),
      ).toBeVisible({ timeout: 5_000 });
    });
  });
}

// ─── Full CRUD sequence ───────────────────────────────────────────────────────

test.describe("Full CRUD: create → drag → edit → delete", () => {
  // ≤640px shows a single column (CO-358) — cross-column drag at that width
  // is covered by the dedicated "Mobile drag" suite below via segment buttons.
  test.beforeEach(async ({ page }) => {
    test.skip((page.viewportSize()?.width ?? 1280) <= 640,
      "single-column mobile board — covered by the Mobile drag suite");
  });

  test("create project task, drag to In Progress, edit title, then delete", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "CRUD flow task",
      status: "todo",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await waitForBoard(page);

    const card = page.locator(`.task-card[data-task-id="${task.id}"]`);
    await expect(card).toBeVisible();

    // Step 2: Drag to "In Progress"
    const inProgressCol = page
      .locator('.kanban-column[data-status="in_progress"]')
      .or(page.locator(".kanban-column").filter({ hasText: "In Progress" }));
    await card.dragTo(inProgressCol);
    await expect(
      inProgressCol.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toBeVisible({ timeout: 5_000 });

    // Step 3: Edit task title via modal
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
    await page.locator("#task-title").fill("CRUD flow task — edited");
    await page.locator("#task-form").dispatchEvent("submit");
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    await expect(
      page.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toContainText("CRUD flow task — edited");

    // Step 4: Delete task via modal
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
    page.on("dialog", (d) => d.accept());
    await page.locator("#btn-delete").click();
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    // Task should be gone from the board
    await expect(
      page.locator(`.task-card[data-task-id="${task.id}"]`),
    ).toHaveCount(0);

    // API confirms deletion
    const getRes = await apiContext.get(
      `/api/projects/${seedProject.key}/tasks/${task.id}`,
    );
    expect(getRes.status()).toBe(404);
  });
});
