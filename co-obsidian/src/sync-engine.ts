// Core sync engine — pull / push / sync
//
// pull():  GET /vault/ listing → compare with local hashes → download changed/new files
// push():  scan .md files → compare hashes → upload changed/new files
// sync():  bidirectional — pull remote first, then push local, last-write-wins on conflict

import type {
  CoPluginSettings,
  SyncMetadata,
  SyncStatus,
  VaultFileInfo,
} from "./types";
import { CoApiClient, CoApiError } from "./api-client";
import {
  coToObsidian,
  obsidianToCo,
  parseFrontmatter,
  extractFrontmatterBlock,
  renderMarkdown,
  parentDataviewLine,
} from "./frontmatter";
import { ensureParentDataviewField } from "./wikilinks";

// We avoid importing obsidian at the top level so tests can mock it cleanly.
// Instead, the engine accepts an adapter interface.

export interface VaultAdapter {
  read(path: string): Promise<string>;
  write(path: string, content: string): Promise<void>;
  exists(path: string): Promise<boolean>;
  list(folder: string): Promise<{ files: string[]; folders: string[] }>;
  remove(path: string): Promise<void>;
  createFolder(path: string): Promise<void>;
}

// Simple SHA-256 hex hash for change detection
async function sha256(text: string): Promise<string> {
  const data = new TextEncoder().encode(text);
  const buf = await crypto.subtle.digest("SHA-256", data);
  return Array.from(new Uint8Array(buf))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// ---------------------------------------------------------------------------
// SyncEngine
// ---------------------------------------------------------------------------

export interface SyncResult {
  pulled: number;
  pushed: number;
  conflicts: number;
  errors: string[];
}

export class SyncEngine {
  private readonly client: CoApiClient;
  private readonly settings: CoPluginSettings;
  private readonly vault: VaultAdapter;
  private readonly syncMetaPath = ".co/sync.json";
  private statusCallback?: (status: SyncStatus) => void;

  constructor(
    settings: CoPluginSettings,
    vault: VaultAdapter
  ) {
    this.settings = settings;
    this.vault = vault;
    this.client = new CoApiClient(settings);
  }

  onStatusChange(cb: (status: SyncStatus) => void): void {
    this.statusCallback = cb;
  }

  private setStatus(status: SyncStatus): void {
    this.statusCallback?.(status);
  }

  // -------------------------------------------------------------------------
  // Sync metadata (.co/sync.json)
  // -------------------------------------------------------------------------

  private async loadMeta(): Promise<SyncMetadata> {
    try {
      if (await this.vault.exists(this.syncMetaPath)) {
        const raw = await this.vault.read(this.syncMetaPath);
        return JSON.parse(raw) as SyncMetadata;
      }
    } catch {
      // corrupt or missing — start fresh
    }
    return { lastSync: "", fileHashes: {}, remoteMtimes: {}, remoteVersion: 0 };
  }

  private async saveMeta(meta: SyncMetadata): Promise<void> {
    if (!(await this.vault.exists(".co"))) {
      await this.vault.createFolder(".co");
    }
    await this.vault.write(this.syncMetaPath, JSON.stringify(meta, null, 2));
  }

  // -------------------------------------------------------------------------
  // pull() — download remote changes to vault
  // -------------------------------------------------------------------------

  async pull(): Promise<SyncResult> {
    this.setStatus({ kind: "syncing" });
    const result: SyncResult = { pulled: 0, pushed: 0, conflicts: 0, errors: [] };

    try {
      const meta = await this.loadMeta();
      const remoteFiles = await this.client.listFiles();

      for (const info of remoteFiles) {
        try {
          await this.pullFile(info, meta, result);
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e);
          result.errors.push(`pull ${info.path}: ${msg}`);
        }
      }

      meta.lastSync = new Date().toISOString();
      await this.saveMeta(meta);

      if (result.conflicts > 0) {
        this.setStatus({ kind: "conflict", count: result.conflicts });
      } else {
        this.setStatus({ kind: "synced", timestamp: new Date() });
      }
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (this.isNetworkError(e)) {
        this.setStatus({ kind: "offline" });
      } else {
        this.setStatus({ kind: "error", message: msg });
      }
      result.errors.push(msg);
    }

    return result;
  }

  private async pullFile(
    info: VaultFileInfo,
    meta: SyncMetadata,
    result: SyncResult
  ): Promise<void> {
    const localPath = this.remoteToLocalPath(info.path);

    // Skip if remote mtime hasn't changed since last pull — avoids extra API call
    const cachedMtime = meta.remoteMtimes?.[localPath];
    if (cachedMtime !== undefined && cachedMtime === info.stat.mtime) {
      return;
    }

    const remoteFile = await this.client.getFile(info.path);

    // Map CO frontmatter to Obsidian frontmatter
    const coFm = remoteFile.frontmatter as Record<string, unknown>;
    const obsFm = coToObsidian(coFm);
    const dataviewLine = parentDataviewLine(coFm);

    let body = remoteFile.content;
    if (dataviewLine) {
      body = ensureParentDataviewField(body, coFm.parent as number);
    }
    const rendered = renderMarkdown(obsFm, body);
    const remoteHash = await sha256(rendered);

    const localHash = meta.fileHashes[localPath];
    if (localHash === remoteHash) {
      // No change
      return;
    }

    // Check for local modifications (conflict detection)
    if (localHash && (await this.vault.exists(localPath))) {
      const localContent = await this.vault.read(localPath);
      const currentLocalHash = await sha256(localContent);
      if (currentLocalHash !== localHash) {
        // Both local and remote changed → conflict
        if (this.settings.conflictMarkers) {
          const conflicted = `<<<<<<< LOCAL\n${localContent}\n=======\n${rendered}\n>>>>>>> REMOTE`;
          await this.vault.write(localPath, conflicted);
        }
        // else: last-write-wins — remote wins on pull
        result.conflicts++;
        return;
      }
    }

    // Ensure parent folder exists
    const folder = localPath.split("/").slice(0, -1).join("/");
    if (folder && !(await this.vault.exists(folder))) {
      await this.vault.createFolder(folder);
    }

    await this.vault.write(localPath, rendered);
    meta.fileHashes[localPath] = remoteHash;
    meta.remoteMtimes = meta.remoteMtimes ?? {};
    meta.remoteMtimes[localPath] = info.stat.mtime;
    result.pulled++;
  }

  // -------------------------------------------------------------------------
  // push() — upload local vault changes to CO
  // -------------------------------------------------------------------------

  async push(): Promise<SyncResult> {
    this.setStatus({ kind: "syncing" });
    const result: SyncResult = { pulled: 0, pushed: 0, conflicts: 0, errors: [] };

    try {
      const meta = await this.loadMeta();
      const localFiles = await this.collectLocalMdFiles();

      for (const localPath of localFiles) {
        try {
          await this.pushFile(localPath, meta, result);
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e);
          result.errors.push(`push ${localPath}: ${msg}`);
        }
      }

      meta.lastSync = new Date().toISOString();
      await this.saveMeta(meta);

      this.setStatus({ kind: "synced", timestamp: new Date() });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (this.isNetworkError(e)) {
        this.setStatus({ kind: "offline" });
      } else {
        this.setStatus({ kind: "error", message: msg });
      }
      result.errors.push(msg);
    }

    return result;
  }

  private async pushFile(
    localPath: string,
    meta: SyncMetadata,
    result: SyncResult
  ): Promise<void> {
    // Skip .co/ metadata and .obsidian/ config
    if (localPath.startsWith(".co/") || localPath.startsWith(".obsidian/")) {
      return;
    }

    const content = await this.vault.read(localPath);
    const currentHash = await sha256(content);
    const lastKnownHash = meta.fileHashes[localPath];

    if (currentHash === lastKnownHash) {
      // No change since last sync
      return;
    }

    // Map Obsidian frontmatter → CO frontmatter before uploading
    const { raw, body } = extractFrontmatterBlock(content);
    let uploadContent = content;
    if (raw) {
      const obsFm = parseFrontmatter(raw);
      const coFm = obsidianToCo(obsFm);
      const { serialiseFrontmatter } = await import("./frontmatter");
      const fmStr = serialiseFrontmatter(coFm);
      uploadContent = `---\n${fmStr}\n---\n${body}`;
    }

    const remotePath = this.localToRemotePath(localPath);
    await this.client.putFile(remotePath, uploadContent);
    meta.fileHashes[localPath] = currentHash;
    result.pushed++;
  }

  // -------------------------------------------------------------------------
  // sync() — bidirectional
  // -------------------------------------------------------------------------

  async sync(): Promise<SyncResult> {
    this.setStatus({ kind: "syncing" });

    // Pull remote changes first, then push local changes.
    const pullResult = await this.pull();
    const pushResult = await this.push();

    return {
      pulled: pullResult.pulled,
      pushed: pushResult.pushed,
      conflicts: pullResult.conflicts + pushResult.conflicts,
      errors: [...pullResult.errors, ...pushResult.errors],
    };
  }

  // -------------------------------------------------------------------------
  // Path mapping: local (vault-relative) ↔ remote (CO path)
  // -------------------------------------------------------------------------

  private remoteToLocalPath(remotePath: string): string {
    // Remote paths are already relative; preserve as-is
    return remotePath;
  }

  private localToRemotePath(localPath: string): string {
    return localPath;
  }

  // -------------------------------------------------------------------------
  // Collect all .md files in vault (excluding .co/ and .obsidian/)
  // -------------------------------------------------------------------------

  private async collectLocalMdFiles(folder = ""): Promise<string[]> {
    const result: string[] = [];
    const listing = await this.vault.list(folder);

    for (const file of listing.files) {
      if (file.endsWith(".md")) {
        result.push(folder ? `${folder}/${file}` : file);
      }
    }

    for (const sub of listing.folders) {
      if (sub === ".co" || sub === ".obsidian") continue;
      const subFiles = await this.collectLocalMdFiles(
        folder ? `${folder}/${sub}` : sub
      );
      result.push(...subFiles);
    }

    return result;
  }

  // -------------------------------------------------------------------------
  // Offline check
  // -------------------------------------------------------------------------

  async isOnline(): Promise<boolean> {
    return this.client.ping();
  }

  /** Returns true for network-level errors (no connection, DNS failure, etc.) */
  private isNetworkError(e: unknown): boolean {
    if (e instanceof TypeError) return true; // fetch() throws TypeError on network failure
    if (e instanceof CoApiError && (e.status === 0 || e.status >= 500)) return true;
    return false;
  }
}
