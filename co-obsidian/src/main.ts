// CO Universe Sync — Obsidian plugin
//
// Syncs a CO universe ↔ Obsidian vault.
// Wikilinks, frontmatter mapping, status bar, ribbon icon, commands.

import {
  App,
  Editor,
  MarkdownView,
  Notice,
  Plugin,
  TFile,
  debounce,
} from "obsidian";

import { CoSettingsTab, DEFAULT_SETTINGS } from "./settings";
import type { CoPluginSettings, SyncStatus } from "./types";
import { SyncEngine, VaultAdapter } from "./sync-engine";
import { StatusBarItem } from "./status-bar";

export default class CoUniverseSyncPlugin extends Plugin {
  settings!: CoPluginSettings;
  private engine!: SyncEngine;
  private statusBarItem!: StatusBarItem;
  private autoSyncIntervalId?: number;

  async onload(): Promise<void> {
    await this.loadSettings();

    // Settings tab
    this.addSettingTab(new CoSettingsTab(this.app, this));

    // Status bar
    const sbEl = this.addStatusBarItem();
    this.statusBarItem = new StatusBarItem(sbEl);

    // Build sync engine with Obsidian vault adapter
    this.engine = this.buildEngine();

    // Ribbon icon — click to sync
    this.addRibbonIcon("refresh-cw", "CO: Sync now", async () => {
      await this.runSync();
    });

    // Register commands
    this.registerCommands();

    // Register link handler so [[CO-XX]] opens the synced file
    this.registerObsidianLinkHandler();

    // Auto-sync on startup (pull)
    if (this.settings.syncIntervalMinutes > 0) {
      await this.runSyncByDirection();
    }

    // Watch file saves — debounced push
    this.registerEvent(
      this.app.vault.on(
        "modify",
        debounce(async (file) => {
          if (!(file instanceof TFile)) return;
          if (!file.path.endsWith(".md")) return;
          if (
            this.settings.syncDirection === "pull" ||
            this.settings.apiToken === ""
          ) return;
          await this.runPush();
        }, 5000)
      )
    );

    // Start auto-sync interval
    this.startAutoSync();
  }

  onunload(): void {
    this.stopAutoSync();
  }

  // -------------------------------------------------------------------------
  // Settings
  // -------------------------------------------------------------------------

  async loadSettings(): Promise<void> {
    this.settings = Object.assign(
      {},
      DEFAULT_SETTINGS,
      await this.loadData()
    ) as CoPluginSettings;
  }

  async saveSettings(): Promise<void> {
    await this.saveData(this.settings);
  }

  // -------------------------------------------------------------------------
  // Sync engine factory
  // -------------------------------------------------------------------------

  private buildEngine(): SyncEngine {
    const adapter = this.buildVaultAdapter();
    const engine = new SyncEngine(this.settings, adapter);
    engine.onStatusChange((status: SyncStatus) => {
      this.statusBarItem.setStatus(status);
    });
    return engine;
  }

  private buildVaultAdapter(): VaultAdapter {
    const vault = this.app.vault;
    return {
      read: async (path: string) => vault.adapter.read(path),
      write: async (path: string, content: string) =>
        vault.adapter.write(path, content),
      exists: async (path: string) => vault.adapter.exists(path),
      list: async (folder: string) => vault.adapter.list(folder),
      remove: async (path: string) => vault.adapter.remove(path),
      createFolder: async (path: string) => {
        try {
          await vault.createFolder(path);
        } catch {
          // Folder may already exist
        }
      },
    };
  }

  // Rebuild engine after settings change
  rebuildEngine(): void {
    this.engine = this.buildEngine();
  }

  // -------------------------------------------------------------------------
  // Sync operations
  // -------------------------------------------------------------------------

  async runPull(): Promise<void> {
    if (!this.validateSettings()) return;
    this.rebuildEngine();
    const result = await this.engine.pull();
    const msg = `CO: pulled ${result.pulled} file(s)${result.conflicts > 0 ? `, ${result.conflicts} conflict(s)` : ""}`;
    new Notice(msg);
    if (result.errors.length > 0) {
      console.error("CO sync errors:", result.errors);
    }
  }

  async runPush(): Promise<void> {
    if (!this.validateSettings()) return;
    this.rebuildEngine();
    const result = await this.engine.push();
    const msg = `CO: pushed ${result.pushed} file(s)`;
    new Notice(msg);
    if (result.errors.length > 0) {
      console.error("CO sync errors:", result.errors);
    }
  }

  async runSync(): Promise<void> {
    if (!this.validateSettings()) return;
    this.rebuildEngine();
    const result = await this.engine.sync();
    const msg =
      `CO: synced — pulled ${result.pulled}, pushed ${result.pushed}` +
      (result.conflicts > 0 ? `, ${result.conflicts} conflict(s)` : "");
    new Notice(msg);
    if (result.errors.length > 0) {
      console.error("CO sync errors:", result.errors);
    }
  }

  async runSyncByDirection(): Promise<void> {
    switch (this.settings.syncDirection) {
      case "pull":
        await this.runPull();
        break;
      case "push":
        await this.runPush();
        break;
      case "bidirectional":
        await this.runSync();
        break;
    }
  }

  private validateSettings(): boolean {
    if (!this.settings.apiToken) {
      new Notice("CO: no API token set — open Settings → CO Universe Sync");
      return false;
    }
    if (!this.settings.universeSlug) {
      new Notice("CO: no universe slug set — open Settings → CO Universe Sync");
      return false;
    }
    return true;
  }

  // -------------------------------------------------------------------------
  // Auto-sync interval
  // -------------------------------------------------------------------------

  startAutoSync(): void {
    this.stopAutoSync();
    if (this.settings.syncIntervalMinutes <= 0) return;
    const ms = this.settings.syncIntervalMinutes * 60 * 1000;
    this.autoSyncIntervalId = window.setInterval(async () => {
      await this.runSyncByDirection();
    }, ms);
    this.registerInterval(this.autoSyncIntervalId);
  }

  stopAutoSync(): void {
    if (this.autoSyncIntervalId !== undefined) {
      window.clearInterval(this.autoSyncIntervalId);
      this.autoSyncIntervalId = undefined;
    }
  }

  restartAutoSync(): void {
    this.startAutoSync();
  }

  // -------------------------------------------------------------------------
  // Commands
  // -------------------------------------------------------------------------

  private registerCommands(): void {
    this.addCommand({
      id: "sync-now",
      name: "Sync now",
      callback: async () => this.runSync(),
    });

    this.addCommand({
      id: "pull-from-co",
      name: "Pull from CO",
      callback: async () => this.runPull(),
    });

    this.addCommand({
      id: "push-to-co",
      name: "Push to CO",
      callback: async () => this.runPush(),
    });

    this.addCommand({
      id: "open-in-co",
      name: "Open in CO",
      checkCallback: (checking: boolean) => {
        const file = this.app.workspace.getActiveFile();
        if (!file) return false;
        if (!checking) this.openInCo(file);
        return true;
      },
    });

    this.addCommand({
      id: "create-task",
      name: "Create task",
      editorCallback: async (editor: Editor, view: MarkdownView) => {
        await this.createTaskFromNote(editor, view);
      },
    });

    this.addCommand({
      id: "link-to-co",
      name: "Link to CO",
      editorCallback: (editor: Editor) => {
        this.insertCoLink(editor);
      },
    });
  }

  // -------------------------------------------------------------------------
  // Command implementations
  // -------------------------------------------------------------------------

  private openInCo(file: TFile): void {
    const { instanceUrl, universeSlug } = this.settings;
    const path = file.path.replace(/\.md$/, "");
    const url = `${instanceUrl}/co/${universeSlug}/${path}`;
    window.open(url, "_blank");
  }

  private async createTaskFromNote(
    editor: Editor,
    view: MarkdownView
  ): Promise<void> {
    if (!this.validateSettings()) return;

    const title =
      view.file?.basename ??
      editor
        .getValue()
        .split("\n")
        .find((l) => l.startsWith("# "))
        ?.slice(2)
        .trim() ??
      "New task";

    const { instanceUrl, universeSlug, apiToken } = this.settings;
    const url = `${instanceUrl}/api/v1/universes/${universeSlug}/vault/${view.file?.path ?? "new-task.md"}`;

    try {
      const res = await fetch(url, {
        method: "PUT",
        headers: {
          Authorization: `Bearer ${apiToken}`,
          "Content-Type": "text/markdown",
        },
        body: editor.getValue(),
      });
      if (res.ok) {
        new Notice(`CO: task "${title}" created ✓`);
      } else {
        new Notice(`CO: failed to create task (${res.status})`);
      }
    } catch {
      new Notice("CO: network error while creating task");
    }
  }

  private insertCoLink(editor: Editor): void {
    // Insert a placeholder wikilink — user can type the id
    const cursor = editor.getCursor();
    editor.replaceRange("[[CO-]]", cursor);
    // Position cursor inside the brackets before the ]]
    editor.setCursor({ line: cursor.line, ch: cursor.ch + 5 });
  }

  // -------------------------------------------------------------------------
  // Link handler — clicking [[CO-XX]] opens synced file
  // -------------------------------------------------------------------------

  private registerObsidianLinkHandler(): void {
    // Register a URI handler for obsidian://co-universe-sync/oauth
    this.registerObsidianProtocolHandler(
      "co-universe-sync",
      async (params) => {
        if (params.action === "oauth" && params.token) {
          this.settings.apiToken = params.token as string;
          await this.saveSettings();
          new Notice("CO: token saved — you are now authenticated ✓");
        }
      }
    );
  }
}
