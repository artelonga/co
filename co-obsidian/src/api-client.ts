// CO Vault API client
//
// Typed wrappers around the REST API defined in vault_routes.rs.
// All requests use Bearer auth (JWT or long-lived API token with co_ prefix).

import type {
  CoPluginSettings,
  VaultFile,
  VaultFileInfo,
  SearchResult,
} from "./types";

export class CoApiError extends Error {
  constructor(
    public readonly status: number,
    message: string
  ) {
    super(message);
    this.name = "CoApiError";
  }
}

export class CoApiClient {
  private readonly baseUrl: string;
  private readonly token: string;
  private readonly slug: string;

  constructor(settings: CoPluginSettings) {
    this.baseUrl = settings.instanceUrl.replace(/\/$/, "");
    this.token = settings.apiToken;
    this.slug = settings.universeSlug;
  }

  private vaultUrl(path = ""): string {
    const base = `${this.baseUrl}/api/v1/universes/${this.slug}/vault`;
    if (!path) return `${base}/`;
    return `${base}/${path.replace(/^\//, "")}`;
  }

  private headers(extra: Record<string, string> = {}): Record<string, string> {
    return {
      Authorization: `Bearer ${this.token}`,
      "Content-Type": "application/json",
      ...extra,
    };
  }

  private async request<T>(
    method: string,
    url: string,
    body?: unknown
  ): Promise<T> {
    const opts: RequestInit = {
      method,
      headers: this.headers(),
    };
    if (body !== undefined) {
      opts.body = JSON.stringify(body);
    }

    const res = await fetch(url, opts);
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw new CoApiError(res.status, `CO API ${res.status}: ${text}`);
    }

    const ct = res.headers.get("content-type") ?? "";
    if (ct.includes("application/json")) {
      return res.json() as Promise<T>;
    }
    return res.text() as unknown as T;
  }

  /** List all files in the vault */
  async listFiles(): Promise<VaultFileInfo[]> {
    const data = await this.request<{ files: VaultFileInfo[] }>(
      "GET",
      this.vaultUrl()
    );
    return data.files;
  }

  /** Get a single file with content and frontmatter */
  async getFile(path: string): Promise<VaultFile> {
    return this.request<VaultFile>("GET", this.vaultUrl(path));
  }

  /** Create or replace a file */
  async putFile(path: string, content: string): Promise<void> {
    const url = this.vaultUrl(path);
    const res = await fetch(url, {
      method: "PUT",
      headers: {
        Authorization: `Bearer ${this.token}`,
        "Content-Type": "text/markdown",
      },
      body: content,
    });
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw new CoApiError(res.status, `CO API ${res.status}: ${text}`);
    }
  }

  /** Delete a file (soft delete to .trash/ by default) */
  async deleteFile(path: string, permanent = false): Promise<void> {
    const url = this.vaultUrl(path) + (permanent ? "?permanent=true" : "");
    const res = await fetch(url, {
      method: "DELETE",
      headers: this.headers(),
    });
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText);
      throw new CoApiError(res.status, `CO API ${res.status}: ${text}`);
    }
  }

  /** Full-text search across vault files */
  async search(query: string): Promise<SearchResult[]> {
    return this.request<SearchResult[]>(
      "POST",
      `${this.baseUrl}/api/v1/universes/${this.slug}/vault/search`,
      { query }
    );
  }

  /** Get all frontmatter tags */
  async getTags(): Promise<Array<{ tag: string; count: number }>> {
    return this.request<Array<{ tag: string; count: number }>>(
      "GET",
      `${this.baseUrl}/api/v1/universes/${this.slug}/vault/tags`
    );
  }

  /** Verify connectivity — resolves true if API responds */
  async ping(): Promise<boolean> {
    try {
      await this.listFiles();
      return true;
    } catch {
      return false;
    }
  }
}
