// Shared types for CO Universe Sync plugin

/** Sync direction setting */
export type SyncDirection = "pull" | "push" | "bidirectional";

/** Sync interval in minutes (0 = off) */
export type SyncInterval = 0 | 5 | 15 | 60;

/** Plugin settings stored in data.json */
export interface CoPluginSettings {
  instanceUrl: string;
  apiToken: string;
  universeSlug: string;
  syncDirection: SyncDirection;
  syncIntervalMinutes: SyncInterval;
  conflictMarkers: boolean;
}

export const DEFAULT_SETTINGS: CoPluginSettings = {
  instanceUrl: "https://artelonga.com.br",
  apiToken: "",
  universeSlug: "",
  syncDirection: "bidirectional",
  syncIntervalMinutes: 15,
  conflictMarkers: false,
};

// ---------------------------------------------------------------------------
// CO Vault API response types (mirrors vault_routes.rs)
// ---------------------------------------------------------------------------

export interface VaultStat {
  ctime: number; // ms since epoch
  mtime: number; // ms since epoch
  size: number;
}

export interface VaultFileInfo {
  path: string;
  stat: VaultStat;
}

export interface VaultFile {
  path: string;
  content: string;
  frontmatter: Record<string, unknown>;
  tags: string[];
  stat: VaultStat;
}

export interface VaultListResponse {
  files: VaultFileInfo[];
}

export interface SearchResult {
  path: string;
  content: string;
  score: number;
}

// ---------------------------------------------------------------------------
// Sync metadata stored in .co/sync.json
// ---------------------------------------------------------------------------

export interface SyncMetadata {
  lastSync: string; // ISO 8601
  fileHashes: Record<string, string>; // path → sha256 hex of local rendered content
  remoteMtimes: Record<string, number>; // path → remote mtime (ms) at last pull
  remoteVersion: number;
}

// ---------------------------------------------------------------------------
// Sync status for status bar
// ---------------------------------------------------------------------------

export type SyncStatus =
  | { kind: "idle" }
  | { kind: "syncing" }
  | { kind: "synced"; timestamp: Date }
  | { kind: "offline" }
  | { kind: "conflict"; count: number }
  | { kind: "error"; message: string };

// ---------------------------------------------------------------------------
// CO content frontmatter (server-side model)
// ---------------------------------------------------------------------------

export interface CoFrontmatter {
  id?: number;
  title?: string;
  type?: string;
  status?: string;
  priority?: string;
  labels?: string[];
  created_at?: string;
  updated_at?: string;
  parent?: number;
  aliases?: string[];
  [key: string]: unknown;
}

/** Obsidian-side frontmatter (what we write into .md files) */
export interface ObsidianFrontmatter {
  id?: number;
  title?: string;
  type?: string;
  status?: string;
  priority?: string;
  tags?: string[];
  created?: string;
  modified?: string;
  parent?: string; // wikilink e.g. "[[CO-21]]"
  aliases?: string[];
  [key: string]: unknown;
}
