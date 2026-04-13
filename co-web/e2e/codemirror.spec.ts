/**
 * CO-33 — CodeMirror 6 editor: initialization, toolbar, preview, persistence
 *
 * The editor bundle (`/shared/editor.bundle.js`) is loaded lazily when a task
 * modal opens.  The tests below exercise:
 *
 *   - Editor mounts in `.co-editor-wrap` / `.cm-editor` when the modal opens
 *   - Typing markdown in `.cm-content` updates the live preview pane
 *   - Bold / Italic / Heading toolbar buttons insert correct markdown syntax
 *   - Save → content persists → reopen → content intact
 */

import { test, expect } from "./fixtures";
import { navigateTo, selectProject, waitForBoard, createTask } from "./helpers";

// Skip all on mobile — sidebar hidden at ≤ 640 px
test.beforeEach(async ({ page }) => {
  const vp = page.viewportSize();
  test.skip(
    !!(vp && vp.width <= 640),
    "Sidebar not available on mobile viewport",
  );
});

// ─── Editor initialisation ────────────────────────────────────────────────────

test.describe("CodeMirror: editor initializes in the task modal", () => {
  test("opening a task modal mounts the editor container", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "CM init task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();

    // Editor bundle loaded and container mounted
    await page.waitForSelector(".co-editor-wrap", {
      state: "visible",
      timeout: 10_000,
    });
    await expect(page.locator(".co-editor-wrap")).toBeVisible();
  });

  test("CM editor node (.cm-editor) is present inside the modal", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "CM node task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();

    await page.waitForSelector(".cm-editor", {
      state: "attached",
      timeout: 10_000,
    });
    await expect(page.locator(".cm-editor").first()).toBeVisible();
  });

  test("editor toolbar renders with Bold, Italic, Heading buttons", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "CM toolbar task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();

    await page.waitForSelector(".co-editor-toolbar", {
      state: "visible",
      timeout: 10_000,
    });

    await expect(
      page.locator(".co-editor-btn[title='Bold (Ctrl+B)']"),
    ).toBeVisible();
    await expect(
      page.locator(".co-editor-btn[title='Italic (Ctrl+I)']"),
    ).toBeVisible();
    await expect(
      page.locator(".co-editor-btn[title='Heading']"),
    ).toBeVisible();
  });
});

// ─── Typing markdown + live preview ──────────────────────────────────────────

test.describe("CodeMirror: typing markdown updates the preview pane", () => {
  test("typing ## Heading renders in the preview pane", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "CM preview task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();

    await page.waitForSelector(".cm-content", {
      state: "visible",
      timeout: 10_000,
    });

    // Ensure the preview pane is visible (toggle it on if hidden on desktop)
    const previewPane = page.locator(".co-editor-right");
    const isHidden = await previewPane
      .evaluate((el) => el.classList.contains("co-preview-hidden"))
      .catch(() => false);
    if (isHidden) {
      const previewBtn = page
        .locator('.co-editor-btn[data-id="preview-toggle"]')
        .or(page.locator(".co-editor-btn").filter({ hasText: "Preview" }));
      if (await previewBtn.count() > 0) await previewBtn.click();
    }

    // Type into the CodeMirror content area
    const cmContent = page.locator(".cm-content").first();
    await cmContent.click();
    await page.keyboard.type("## Hello from preview");

    // Preview pane should render the heading text
    await expect(previewPane).toContainText("Hello from preview", {
      timeout: 3_000,
    });
  });
});

// ─── Toolbar buttons ──────────────────────────────────────────────────────────

test.describe("CodeMirror toolbar: Bold, Italic, Heading insert correct syntax", () => {
  async function openEditorForTask(
    page: import("@playwright/test").Page,
    taskId: number,
  ) {
    await page.locator(`.task-card[data-task-id="${taskId}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
    await page.waitForSelector(".cm-content", {
      state: "visible",
      timeout: 10_000,
    });
  }

  test("Bold button wraps selected text with **...**", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Bold btn task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await openEditorForTask(page, task.id);

    const cmContent = page.locator(".cm-content").first();
    await cmContent.click();
    await page.keyboard.type("bold text");
    // Select all text in editor
    await page.keyboard.press("Control+a");

    await page.locator(".co-editor-btn[title='Bold (Ctrl+B)']").click();

    // Hidden textarea syncs editor value
    const textareaVal = await page.locator("#task-description").inputValue();
    expect(textareaVal).toMatch(/\*\*/);
  });

  test("Italic button wraps selected text with *...*", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Italic btn task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await openEditorForTask(page, task.id);

    const cmContent = page.locator(".cm-content").first();
    await cmContent.click();
    await page.keyboard.type("italic text");
    await page.keyboard.press("Control+a");

    await page.locator(".co-editor-btn[title='Italic (Ctrl+I)']").click();

    const textareaVal = await page.locator("#task-description").inputValue();
    expect(textareaVal).toMatch(/\*/);
  });

  test("Heading button prepends ## to the current line", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Heading btn task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await openEditorForTask(page, task.id);

    const cmContent = page.locator(".cm-content").first();
    await cmContent.click();
    await page.keyboard.type("heading text");

    await page.locator(".co-editor-btn[title='Heading']").click();

    const textareaVal = await page.locator("#task-description").inputValue();
    expect(textareaVal).toMatch(/##/);
  });
});

// ─── Save → persist → reopen ──────────────────────────────────────────────────

test.describe("CodeMirror: save persists content across modal open/close", () => {
  test("typed description survives save → close → reopen", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, {
      title: "Persist CM task",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();

    await page.waitForSelector(".cm-content", {
      state: "visible",
      timeout: 10_000,
    });

    const contentToType = "Persisted description via CM editor";

    const cmContent = page.locator(".cm-content").first();
    await cmContent.click();
    // Clear any existing content
    await page.keyboard.press("Control+a");
    await page.keyboard.type(contentToType);

    // Save
    await page.locator("#task-form").dispatchEvent("submit");
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    // Reopen
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();

    // Content should be present in the hidden textarea (source of truth)
    const textareaVal = await page.locator("#task-description").inputValue();
    expect(textareaVal).toContain(contentToType);
  });

  test("content saved via API is shown in the editor when the modal opens", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    // Create task with description via API
    const task = await createTask(apiContext, seedProject.key, {
      title: "Pre-filled CM task",
      description: "Pre-existing description content",
    });

    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();

    await page.waitForSelector(".co-editor-wrap", {
      state: "visible",
      timeout: 10_000,
    });

    // Hidden textarea should contain the pre-filled content
    const textareaVal = await page.locator("#task-description").inputValue();
    expect(textareaVal).toContain("Pre-existing description content");
  });
});
