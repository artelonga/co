// Integration test — mock CO API → sync → verify vault files

import { SyncEngine, VaultAdapter } from "../src/sync-engine";
import type { CoPluginSettings, VaultFileInfo } from "../src/types";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeSettings(overrides: Partial<CoPluginSettings> = {}): CoPluginSettings {
  return {
    instanceUrl: "https://test.co",
    apiToken: "co_test_token",
    universeSlug: "test-universe",
    syncDirection: "bidirectional",
    syncIntervalMinutes: 0,
    conflictMarkers: false,
    ...overrides,
  };
}

/** In-memory vault adapter for testing */
class MemoryVaultAdapter implements VaultAdapter {
  files: Map<string, string> = new Map();

  async read(path: string): Promise<string> {
    const content = this.files.get(path);
    if (content === undefined) throw new Error(`File not found: ${path}`);
    return content;
  }

  async write(path: string, content: string): Promise<void> {
    this.files.set(path, content);
  }

  async exists(path: string): Promise<boolean> {
    // Also check if it's a "folder" prefix
    return this.files.has(path) || [...this.files.keys()].some((k) => k.startsWith(path + "/"));
  }

  async list(folder: string): Promise<{ files: string[]; folders: string[] }> {
    const prefix = folder ? folder + "/" : "";
    const files: string[] = [];
    const folders = new Set<string>();

    for (const key of this.files.keys()) {
      if (!key.startsWith(prefix)) continue;
      const rest = key.slice(prefix.length);
      if (!rest) continue;
      const parts = rest.split("/");
      if (parts.length === 1) {
        files.push(parts[0]);
      } else {
        folders.add(parts[0]);
      }
    }

    return { files, folders: [...folders] };
  }

  async remove(path: string): Promise<void> {
    this.files.delete(path);
  }

  async createFolder(_path: string): Promise<void> {
    // No-op — in-memory adapter doesn't need real folder creation
  }
}

// ---------------------------------------------------------------------------
// Mock fetch for CO API
// ---------------------------------------------------------------------------

type FetchHandler = (url: string, opts: RequestInit) => Response;

function setupFetchMock(handler: FetchHandler): void {
  global.fetch = jest.fn((url: string, opts: RequestInit = {}) =>
    Promise.resolve(handler(url, opts))
  ) as jest.MockedFunction<typeof fetch>;
}

function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: { "content-type": "application/json" },
  });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

const REMOTE_FILES: VaultFileInfo[] = [
  { path: "content/hello.md", stat: { ctime: 1000, mtime: 2000, size: 100 } },
  { path: "README.md", stat: { ctime: 1000, mtime: 2000, size: 50 } },
];

const REMOTE_FILE_CONTENT = {
  "content/hello.md": {
    path: "content/hello.md",
    content: "Hello from CO.",
    frontmatter: {
      id: 1,
      title: "Hello",
      labels: ["test"],
      created_at: "2026-04-06T00:00:00Z",
      updated_at: "2026-04-06T00:00:00Z",
      parent: 20,
    },
    tags: ["test"],
    stat: { ctime: 1000, mtime: 2000, size: 100 },
  },
  "README.md": {
    path: "README.md",
    content: "Universe description.",
    frontmatter: {},
    tags: [],
    stat: { ctime: 1000, mtime: 2000, size: 50 },
  },
};

beforeEach(() => {
  // crypto.subtle is available in Node 18+ global scope in tests,
  // but TextEncoder may need polyfilling in older setups.
  if (!global.crypto) {
    // @ts-expect-error polyfill
    global.crypto = require("crypto").webcrypto;
  }
});

describe("SyncEngine.pull()", () => {
  it("downloads all remote files into the vault", async () => {
    const vault = new MemoryVaultAdapter();

    setupFetchMock((url) => {
      if (url.endsWith("/vault/")) return jsonResponse({ files: REMOTE_FILES });
      for (const [path, data] of Object.entries(REMOTE_FILE_CONTENT)) {
        if (url.endsWith(`/vault/${path}`)) return jsonResponse(data);
      }
      return jsonResponse({ error: "not found" }, 404);
    });

    const engine = new SyncEngine(makeSettings(), vault);
    const result = await engine.pull();

    expect(result.pulled).toBe(2);
    expect(result.errors).toHaveLength(0);
    expect(vault.files.has("content/hello.md")).toBe(true);
    expect(vault.files.has("README.md")).toBe(true);
  });

  it("maps CO frontmatter to Obsidian frontmatter in pulled files", async () => {
    const vault = new MemoryVaultAdapter();

    setupFetchMock((url) => {
      if (url.endsWith("/vault/")) return jsonResponse({ files: [REMOTE_FILES[0]] });
      if (url.includes("content/hello.md"))
        return jsonResponse(REMOTE_FILE_CONTENT["content/hello.md"]);
      return jsonResponse({ error: "not found" }, 404);
    });

    const engine = new SyncEngine(makeSettings(), vault);
    await engine.pull();

    const content = vault.files.get("content/hello.md") ?? "";
    // labels → tags
    expect(content).toContain("tags:");
    expect(content).toContain("  - test");
    // created_at → created (ISO timestamp — unquoted)
    expect(content).toContain("created: 2026-04-06T00:00:00Z");
    // parent: 20 → parent: [[CO-20]] (wikilink, quoted because starts with "[")
    expect(content).toMatch(/parent: "?\[\[CO-20\]\]"?/);
    // Dataview field
    expect(content).toContain("parent:: [[CO-20]]");
  });

  it("does not re-download unchanged files", async () => {
    const vault = new MemoryVaultAdapter();

    let fetchCount = 0;
    setupFetchMock((url) => {
      if (url.endsWith("/vault/")) return jsonResponse({ files: [REMOTE_FILES[0]] });
      fetchCount++;
      return jsonResponse(REMOTE_FILE_CONTENT["content/hello.md"]);
    });

    const engine = new SyncEngine(makeSettings(), vault);
    // First pull
    await engine.pull();
    const firstCount = fetchCount;
    // Second pull — nothing changed
    await engine.pull();
    // File fetch should not increase (only listing call)
    expect(fetchCount).toBe(firstCount);
  });

  it("sets status to offline when server is unreachable", async () => {
    const vault = new MemoryVaultAdapter();
    setupFetchMock(() => {
      throw new TypeError("fetch failed");
    });

    const statuses: string[] = [];
    const engine = new SyncEngine(makeSettings(), vault);
    engine.onStatusChange((s) => statuses.push(s.kind));

    await engine.pull();
    expect(statuses).toContain("offline");
  });
});

describe("SyncEngine.push()", () => {
  it("uploads local .md files to CO", async () => {
    const vault = new MemoryVaultAdapter();
    vault.files.set(
      "content/local-note.md",
      `---\ntitle: Local Note\ntags:\n  - local\n---\nLocal content.`
    );

    const puts: Array<{ url: string; body: string }> = [];
    setupFetchMock((url, opts) => {
      if (opts.method === "PUT") {
        puts.push({ url, body: (opts.body as string) ?? "" });
        return new Response(null, { status: 204 });
      }
      return jsonResponse({ error: "unexpected" }, 400);
    });

    const engine = new SyncEngine(makeSettings(), vault);
    const result = await engine.push();

    expect(result.pushed).toBe(1);
    expect(puts).toHaveLength(1);
    expect(puts[0].url).toContain("content/local-note.md");
  });

  it("maps Obsidian frontmatter to CO frontmatter when uploading", async () => {
    const vault = new MemoryVaultAdapter();
    vault.files.set(
      "note.md",
      `---\ntitle: My Note\ntags:\n  - rust\ncreated: 2026-04-06T00:00:00Z\n---\nBody.`
    );

    const puts: string[] = [];
    setupFetchMock((url, opts) => {
      if (opts.method === "PUT") {
        puts.push((opts.body as string) ?? "");
        return new Response(null, { status: 204 });
      }
      return jsonResponse({ error: "unexpected" }, 400);
    });

    const engine = new SyncEngine(makeSettings(), vault);
    await engine.push();

    expect(puts).toHaveLength(1);
    const uploaded = puts[0];
    // tags → labels
    expect(uploaded).toContain("labels:");
    expect(uploaded).toContain("  - rust");
    // created → created_at (ISO timestamp — unquoted)
    expect(uploaded).toContain("created_at: 2026-04-06T00:00:00Z");
  });

  it("skips .co/ metadata files", async () => {
    const vault = new MemoryVaultAdapter();
    vault.files.set(".co/sync.json", '{"lastSync":"","fileHashes":{},"remoteVersion":0}');
    vault.files.set("content/real.md", "---\ntitle: Real\n---\nContent.");

    const puts: string[] = [];
    setupFetchMock((url, opts) => {
      if (opts.method === "PUT") {
        puts.push(url);
        return new Response(null, { status: 204 });
      }
      return jsonResponse({ error: "unexpected" }, 400);
    });

    const engine = new SyncEngine(makeSettings(), vault);
    await engine.push();

    // Ensure no .co/ metadata paths were uploaded (check URL path, not domain)
    const urlPaths = puts.map((u) => new URL(u).pathname);
    expect(urlPaths.every((p) => !p.includes("/.co/"))).toBe(true);
  });
});

describe("SyncEngine.sync()", () => {
  it("pulls then pushes in bidirectional mode", async () => {
    const vault = new MemoryVaultAdapter();
    vault.files.set("local.md", "---\ntitle: Local\n---\nLocal.");

    const puts: string[] = [];
    setupFetchMock((url, opts) => {
      if (url.endsWith("/vault/")) return jsonResponse({ files: [REMOTE_FILES[1]] });
      if (url.includes("README.md") && opts.method !== "PUT")
        return jsonResponse(REMOTE_FILE_CONTENT["README.md"]);
      if (opts.method === "PUT") {
        puts.push(url);
        return new Response(null, { status: 204 });
      }
      return jsonResponse({ error: "not found" }, 404);
    });

    const engine = new SyncEngine(makeSettings(), vault);
    const result = await engine.sync();

    expect(result.pulled).toBeGreaterThanOrEqual(1); // README.md pulled
    expect(result.pushed).toBeGreaterThanOrEqual(1); // local.md pushed
  });
});
