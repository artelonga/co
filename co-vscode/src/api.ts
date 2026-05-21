// CO Vault API client for the VS Code extension
// Mirrors api-client.ts from co-obsidian, adapted for the extension host.

import type {
  MeUniversesResponse,
  VaultFile,
  VaultFileInfo,
} from "./types";

export class CoApiError extends Error {
  constructor(
    public readonly status: number,
    message: string,
  ) {
    super(message);
    this.name = "CoApiError";
  }
}

export class CoApiClient {
  constructor(
    private readonly baseUrl: string,
    private readonly token: string,
  ) {}

  private authHeaders(): Record<string, string> {
    return { Authorization: `Bearer ${this.token}` };
  }

  private vaultUrl(slug: string, vaultPath = ""): string {
    const base = `${this.baseUrl}/api/v1/universes/${slug}/vault`;
    if (!vaultPath) return `${base}/`;
    return `${base}/${vaultPath.replace(/^\//, "")}`;
  }

  private async request<T>(
    method: string,
    url: string,
    body?: string,
    contentType = "application/json",
  ): Promise<T> {
    const headers: Record<string, string> = { ...this.authHeaders() };
    if (body !== undefined) headers["Content-Type"] = contentType;

    const res = await fetch(url, { method, headers, body });
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw new CoApiError(res.status, `CO ${res.status}: ${text}`);
    }
    const ct = res.headers.get("content-type") ?? "";
    if (ct.includes("application/json")) return res.json() as Promise<T>;
    return res.text() as unknown as T;
  }

  /** List all files in the universe vault */
  async listFiles(slug: string): Promise<VaultFileInfo[]> {
    const data = await this.request<{ files: VaultFileInfo[] }>(
      "GET",
      this.vaultUrl(slug),
    );
    return data.files;
  }

  /** Get a single file with content + frontmatter */
  async getFile(slug: string, filePath: string): Promise<VaultFile> {
    return this.request<VaultFile>("GET", this.vaultUrl(slug, filePath));
  }

  /** Create or replace a file (plain markdown text) */
  async putFile(
    slug: string,
    filePath: string,
    content: string,
  ): Promise<void> {
    await this.request<void>(
      "PUT",
      this.vaultUrl(slug, filePath),
      content,
      "text/markdown",
    );
  }

  /** Delete a file */
  async deleteFile(
    slug: string,
    filePath: string,
    permanent = false,
  ): Promise<void> {
    const url =
      this.vaultUrl(slug, filePath) + (permanent ? "?permanent=true" : "");
    const res = await fetch(url, {
      method: "DELETE",
      headers: this.authHeaders(),
    });
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw new CoApiError(res.status, `CO ${res.status}: ${text}`);
    }
  }

  /** List universes the authenticated user can access */
  async getMyUniverses(): Promise<MeUniversesResponse> {
    return this.request<MeUniversesResponse>(
      "GET",
      `${this.baseUrl}/api/v1/me/universes`,
    );
  }

  /** Verify connectivity */
  async ping(): Promise<boolean> {
    try {
      const res = await fetch(`${this.baseUrl}/api/health`, {
        headers: this.authHeaders(),
      });
      return res.ok;
    } catch {
      return false;
    }
  }
}
