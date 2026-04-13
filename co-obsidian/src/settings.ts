// Plugin settings and settings tab
//
// Settings stored in Obsidian's data.json (not in vault files).
// Includes: CO instance URL, API token, universe slug, sync direction, interval.

import { App, Notice, PluginSettingTab, Setting } from "obsidian";
import type CoUniverseSyncPlugin from "./main";
import { DEFAULT_SETTINGS } from "./types";
import type { CoPluginSettings, SyncDirection, SyncInterval } from "./types";
import { CoApiClient } from "./api-client";

export { DEFAULT_SETTINGS };
export type { CoPluginSettings };

export class CoSettingsTab extends PluginSettingTab {
  private readonly plugin: CoUniverseSyncPlugin;

  constructor(app: App, plugin: CoUniverseSyncPlugin) {
    super(app, plugin);
    this.plugin = plugin;
  }

  display(): void {
    const { containerEl } = this;
    containerEl.empty();

    containerEl.createEl("h2", { text: "CO Universe Sync" });

    // -------------------------------------------------------------------------
    // Instance URL
    // -------------------------------------------------------------------------
    new Setting(containerEl)
      .setName("CO instance URL")
      .setDesc("Base URL of your CO server (e.g. https://artelonga.com.br)")
      .addText((text) =>
        text
          .setPlaceholder("https://artelonga.com.br")
          .setValue(this.plugin.settings.instanceUrl)
          .onChange(async (value) => {
            this.plugin.settings.instanceUrl = value.trim().replace(/\/$/, "");
            await this.plugin.saveSettings();
          })
      );

    // -------------------------------------------------------------------------
    // Universe slug
    // -------------------------------------------------------------------------
    new Setting(containerEl)
      .setName("Universe slug")
      .setDesc("The slug of the CO universe to sync (e.g. my-notes)")
      .addText((text) =>
        text
          .setPlaceholder("my-notes")
          .setValue(this.plugin.settings.universeSlug)
          .onChange(async (value) => {
            this.plugin.settings.universeSlug = value.trim().toLowerCase();
            await this.plugin.saveSettings();
          })
      );

    // -------------------------------------------------------------------------
    // API Token
    // -------------------------------------------------------------------------
    containerEl.createEl("h3", { text: "Authentication" });

    new Setting(containerEl)
      .setName("API token")
      .setDesc(
        "Paste a long-lived API token from CO (Settings → API Tokens). " +
          "Token is stored securely in Obsidian's plugin data."
      )
      .addText((text) => {
        text
          .setPlaceholder("co_...")
          .setValue(this.plugin.settings.apiToken)
          .onChange(async (value) => {
            this.plugin.settings.apiToken = value.trim();
            await this.plugin.saveSettings();
          });
        text.inputEl.type = "password";
      });

    // OAuth login button
    new Setting(containerEl)
      .setName("Login with CO")
      .setDesc(
        "Open your browser to authenticate and receive a token automatically."
      )
      .addButton((btn) =>
        btn
          .setButtonText("Login with CO")
          .setCta()
          .onClick(() => this.handleOAuthLogin())
      );

    // Verify token button
    new Setting(containerEl)
      .setName("Verify connection")
      .setDesc("Test the current token and universe slug.")
      .addButton((btn) =>
        btn.setButtonText("Test connection").onClick(async () => {
          const ok = await this.testConnection();
          new Notice(ok ? "CO: connection OK ✓" : "CO: connection failed ✗");
        })
      );

    // -------------------------------------------------------------------------
    // Sync direction
    // -------------------------------------------------------------------------
    containerEl.createEl("h3", { text: "Sync options" });

    new Setting(containerEl)
      .setName("Sync direction")
      .setDesc("How to reconcile differences between vault and CO universe.")
      .addDropdown((drop) =>
        drop
          .addOption("pull", "Pull only (CO → vault)")
          .addOption("push", "Push only (vault → CO)")
          .addOption("bidirectional", "Bidirectional (last-write-wins)")
          .setValue(this.plugin.settings.syncDirection)
          .onChange(async (value) => {
            this.plugin.settings.syncDirection = value as SyncDirection;
            await this.plugin.saveSettings();
          })
      );

    // -------------------------------------------------------------------------
    // Sync interval
    // -------------------------------------------------------------------------
    new Setting(containerEl)
      .setName("Auto-sync interval")
      .setDesc("How often to sync automatically (0 = disabled).")
      .addDropdown((drop) =>
        drop
          .addOption("0", "Disabled")
          .addOption("5", "Every 5 minutes")
          .addOption("15", "Every 15 minutes")
          .addOption("60", "Every hour")
          .setValue(String(this.plugin.settings.syncIntervalMinutes))
          .onChange(async (value) => {
            this.plugin.settings.syncIntervalMinutes = parseInt(
              value,
              10
            ) as SyncInterval;
            await this.plugin.saveSettings();
            this.plugin.restartAutoSync();
          })
      );

    // -------------------------------------------------------------------------
    // Conflict markers
    // -------------------------------------------------------------------------
    new Setting(containerEl)
      .setName("Conflict markers")
      .setDesc(
        "On bidirectional conflict, write <<<<<<</=======/>>>>>>> markers into the file " +
          "instead of silently letting remote win."
      )
      .addToggle((toggle) =>
        toggle
          .setValue(this.plugin.settings.conflictMarkers)
          .onChange(async (value) => {
            this.plugin.settings.conflictMarkers = value;
            await this.plugin.saveSettings();
          })
      );
  }

  private async testConnection(): Promise<boolean> {
    try {
      const client = new CoApiClient(this.plugin.settings);
      return await client.ping();
    } catch {
      return false;
    }
  }

  private handleOAuthLogin(): void {
    const { instanceUrl } = this.plugin.settings;
    const callbackUrl = "obsidian://co-universe-sync/oauth";
    const authUrl = `${instanceUrl}/api/v1/auth/oauth?redirect_uri=${encodeURIComponent(callbackUrl)}&plugin=obsidian`;
    // Open browser — Obsidian provides window.open in desktop
    window.open(authUrl, "_blank");
    new Notice(
      "CO: browser opened for authentication. Paste the token once you receive it."
    );
  }
}
