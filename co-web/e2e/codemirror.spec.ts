/**
 * CodeMirror — thinned (CO-302): init + preview + persistence.
 * Toolbar button syntax and bold/italic/heading details moved to component tests.
 */

import { test, expect } from "./fixtures";
import { navigateTo, selectProject, createTask } from "./helpers";

test.describe("CodeMirror: init", () => {
  test("opening a task modal mounts .co-editor-wrap", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, { title: "CM init" });
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await expect(page.locator("#modal-overlay")).toBeVisible();
    await page.waitForSelector(".co-editor-wrap", { state: "visible", timeout: 10_000 });
    await expect(page.locator(".cm-editor").first()).toBeVisible();
  });
});

test.describe("CodeMirror: preview", () => {
  test("typing ## Heading renders in preview pane", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, { title: "CM preview" });
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await page.waitForSelector(".cm-content", { state: "visible", timeout: 10_000 });

    const previewPane = page.locator(".co-editor-right");
    const isHidden = await previewPane
      .evaluate((el) => el.classList.contains("co-preview-hidden"))
      .catch(() => false);
    if (isHidden) {
      const btn = page.locator('.co-editor-btn[data-id="preview-toggle"]')
        .or(page.locator(".co-editor-btn").filter({ hasText: "Preview" }));
      if (await btn.count() > 0) await btn.click();
    }

    await page.locator(".cm-content").first().click();
    await page.keyboard.type("## Hello from preview");
    await expect(previewPane).toContainText("Hello from preview", { timeout: 3_000 });
  });
});

test.describe("CodeMirror: persistence", () => {
  test("typed description persists after save → close → reopen", async ({
    page,
    apiContext,
    seedProject,
  }) => {
    const task = await createTask(apiContext, seedProject.key, { title: "CM persist" });
    await navigateTo(page, "/");
    await selectProject(page, seedProject.key);
    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    await page.waitForSelector(".cm-content", { state: "visible", timeout: 10_000 });

    const content = "Persisted via CM editor";
    await page.locator(".cm-content").first().click();
    await page.keyboard.press("Control+a");
    await page.keyboard.type(content);

    await page.locator("#task-form").dispatchEvent("submit");
    await expect(page.locator("#modal-overlay")).not.toBeVisible();

    await page.locator(`.task-card[data-task-id="${task.id}"]`).click();
    const val = await page.locator("#task-description").inputValue();
    expect(val).toContain(content);
  });
});
