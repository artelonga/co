// CO FileSystemProvider — maps co://{slug}/{path} to the CO Vault API.
// Registered for the "co" URI scheme so VS Code treats CO universes as
// remote file systems.

import * as vscode from "vscode";

import { CoApiClient, CoApiError } from "./api";
import type { VaultFileInfo } from "./types";

// ─── Directory-tree cache ────────────────────────────────────────────────────

interface DirTree {
  files: Map<string, VaultFileInfo>; // path → stat
  dirs: Set<string>; // derived parent directories
}

function buildTree(files: VaultFileInfo[]): DirTree {
  const fileMap = new Map<string, VaultFileInfo>();
  const dirs = new Set<string>();

  for (const f of files) {
    fileMap.set(f.path, f);
    const parts = f.path.split("/");
    for (let i = 1; i < parts.length; i++) {
      dirs.add(parts.slice(0, i).join("/"));
    }
  }

  return { files: fileMap, dirs };
}

// ─── URI helpers ─────────────────────────────────────────────────────────────

export interface ParsedUri {
  slug: string;
  filePath: string; // vault-relative path, never starts with "/"
}

export function parseUri(uri: vscode.Uri): ParsedUri {
  const slug = uri.authority;
  // uri.path starts with "/" — strip it for the vault API
  const filePath = uri.path.replace(/^\//, "");
  return { slug, filePath };
}

/** Returns the path component relative to dir, or null if not under dir. */
export function relativeTo(dir: string, filePath: string): string | null {
  if (dir === "") return filePath;
  if (!filePath.startsWith(`${dir}/`)) return null;
  return filePath.slice(dir.length + 1);
}

// ─── Provider ────────────────────────────────────────────────────────────────

export class CoFileSystemProvider implements vscode.FileSystemProvider {
  private readonly _emitter = new vscode.EventEmitter<
    vscode.FileChangeEvent[]
  >();
  readonly onDidChangeFile: vscode.Event<vscode.FileChangeEvent[]> =
    this._emitter.event;

  private readonly cache = new Map<
    string,
    { tree: DirTree; expiry: number }
  >();
  private readonly cacheTtlMs = 30_000;

  constructor(readonly client: CoApiClient) {}

  watch(): vscode.Disposable {
    return new vscode.Disposable(() => {});
  }

  async stat(uri: vscode.Uri): Promise<vscode.FileStat> {
    const { slug, filePath } = parseUri(uri);

    if (!filePath) {
      return { type: vscode.FileType.Directory, ctime: 0, mtime: 0, size: 0 };
    }

    const tree = await this.getTree(slug);

    if (tree.dirs.has(filePath)) {
      return { type: vscode.FileType.Directory, ctime: 0, mtime: 0, size: 0 };
    }

    const info = tree.files.get(filePath);
    if (info) {
      return {
        type: vscode.FileType.File,
        ctime: info.stat.ctime,
        mtime: info.stat.mtime,
        size: info.stat.size,
      };
    }

    throw vscode.FileSystemError.FileNotFound(uri);
  }

  async readDirectory(
    uri: vscode.Uri,
  ): Promise<[string, vscode.FileType][]> {
    const { slug, filePath } = parseUri(uri);
    const tree = await this.getTree(slug);

    const entries = new Map<string, vscode.FileType>();

    for (const [p] of tree.files) {
      const rel = relativeTo(filePath, p);
      if (rel === null) continue;
      const slashIdx = rel.indexOf("/");
      if (slashIdx === -1) {
        entries.set(rel, vscode.FileType.File);
      } else {
        entries.set(rel.slice(0, slashIdx), vscode.FileType.Directory);
      }
    }

    return Array.from(entries.entries());
  }

  async readFile(uri: vscode.Uri): Promise<Uint8Array> {
    const { slug, filePath } = parseUri(uri);
    try {
      const file = await this.client.getFile(slug, filePath);
      return Buffer.from(file.content, "utf-8");
    } catch (e) {
      if (e instanceof CoApiError && e.status === 404) {
        throw vscode.FileSystemError.FileNotFound(uri);
      }
      throw e;
    }
  }

  async writeFile(
    uri: vscode.Uri,
    content: Uint8Array,
    _options: { create: boolean; overwrite: boolean },
  ): Promise<void> {
    const { slug, filePath } = parseUri(uri);
    await this.client.putFile(
      slug,
      filePath,
      Buffer.from(content).toString("utf-8"),
    );
    this.invalidateCache(slug);
    this._emitter.fire([{ type: vscode.FileChangeType.Changed, uri }]);
  }

  async delete(
    uri: vscode.Uri,
    _options: { recursive: boolean },
  ): Promise<void> {
    const { slug, filePath } = parseUri(uri);
    await this.client.deleteFile(slug, filePath);
    this.invalidateCache(slug);
    this._emitter.fire([{ type: vscode.FileChangeType.Deleted, uri }]);
  }

  async createDirectory(uri: vscode.Uri): Promise<void> {
    // CO Vault is file-centric; create a .keep placeholder so the directory exists.
    const keepUri = vscode.Uri.parse(
      `co://${uri.authority}${uri.path}/.keep`,
    );
    await this.writeFile(keepUri, new TextEncoder().encode(""), {
      create: true,
      overwrite: false,
    });
  }

  async rename(
    oldUri: vscode.Uri,
    newUri: vscode.Uri,
    _options: { overwrite: boolean },
  ): Promise<void> {
    const content = await this.readFile(oldUri);
    await this.writeFile(newUri, content, { create: true, overwrite: true });
    await this.delete(oldUri, { recursive: false });
  }

  /** Exposed for the wikilink completion provider. */
  async listFilesForSlug(slug: string): Promise<VaultFileInfo[]> {
    const tree = await this.getTree(slug);
    return Array.from(tree.files.values());
  }

  private async getTree(slug: string): Promise<DirTree> {
    const cached = this.cache.get(slug);
    if (cached && Date.now() < cached.expiry) return cached.tree;

    const files = await this.client.listFiles(slug);
    const tree = buildTree(files);
    this.cache.set(slug, { tree, expiry: Date.now() + this.cacheTtlMs });
    return tree;
  }

  private invalidateCache(slug: string): void {
    this.cache.delete(slug);
  }
}
