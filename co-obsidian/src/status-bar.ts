// Status bar indicator
//
// Displays current sync state in Obsidian's status bar:
//   "CO: synced ✓"    — last sync successful
//   "CO: syncing..."  — sync in progress
//   "CO: offline"     — server unreachable
//   "CO: 3 conflicts" — conflicts detected
//   "CO: error"       — unexpected error

import type { SyncStatus } from "./types";

export class StatusBarItem {
  private readonly el: HTMLElement;

  constructor(el: HTMLElement) {
    this.el = el;
    this.setStatus({ kind: "idle" });
  }

  setStatus(status: SyncStatus): void {
    this.el.setText(this.label(status));
    this.el.title = this.tooltip(status);
    this.el.className = `plugin-co-status plugin-co-status--${status.kind}`;
  }

  private label(status: SyncStatus): string {
    switch (status.kind) {
      case "idle":
        return "CO";
      case "syncing":
        return "CO: syncing\u2026";
      case "synced":
        return "CO: synced \u2713";
      case "offline":
        return "CO: offline";
      case "conflict":
        return `CO: ${status.count} conflict${status.count === 1 ? "" : "s"}`;
      case "error":
        return "CO: error";
    }
  }

  private tooltip(status: SyncStatus): string {
    switch (status.kind) {
      case "idle":
        return "CO Universe Sync — not yet synced";
      case "syncing":
        return "CO Universe Sync — sync in progress";
      case "synced":
        return `CO Universe Sync — last synced at ${status.timestamp.toLocaleTimeString()}`;
      case "offline":
        return "CO Universe Sync — server unreachable";
      case "conflict":
        return `CO Universe Sync — ${status.count} file(s) have conflicts`;
      case "error":
        return `CO Universe Sync — error: ${status.message}`;
    }
  }
}
