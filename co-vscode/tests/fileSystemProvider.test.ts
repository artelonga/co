import { FileType } from "vscode";

import { CoApiClient } from "../src/api";
import {
  CoFileSystemProvider,
  parseUri,
  relativeTo,
} from "../src/coFileSystemProvider";
import { Uri } from "./__mocks__/vscode";

// ─── parseUri ────────────────────────────────────────────────────────────────

describe("parseUri", () => {
  it("extracts slug and file path", () => {
    const uri = Uri.parse("co://my-universe/projects/CO/1.md");
    const { slug, filePath } = parseUri(uri as unknown as import("vscode").Uri);
    expect(slug).toBe("my-universe");
    expect(filePath).toBe("projects/CO/1.md");
  });

  it("returns empty filePath for root URI", () => {
    const uri = Uri.parse("co://my-universe/");
    const { slug, filePath } = parseUri(uri as unknown as import("vscode").Uri);
    expect(slug).toBe("my-universe");
    expect(filePath).toBe("");
  });
});

// ─── relativeTo ──────────────────────────────────────────────────────────────

describe("relativeTo", () => {
  it("returns filename when dir is empty (root)", () => {
    expect(relativeTo("", "notes/foo.md")).toBe("notes/foo.md");
  });

  it("returns relative path inside a directory", () => {
    expect(relativeTo("notes", "notes/foo.md")).toBe("foo.md");
  });

  it("returns null when path is not under dir", () => {
    expect(relativeTo("notes", "projects/bar.md")).toBeNull();
  });

  it("does not match partial directory names", () => {
    expect(relativeTo("note", "notes/foo.md")).toBeNull();
  });
});

// ─── CoFileSystemProvider ────────────────────────────────────────────────────

function makeClient(files: Array<{ path: string; ctime?: number; mtime?: number; size?: number }>): CoApiClient {
  const client = {
    listFiles: jest.fn().mockResolvedValue(
      files.map((f) => ({
        path: f.path,
        stat: { ctime: f.ctime ?? 0, mtime: f.mtime ?? 0, size: f.size ?? 0 },
      })),
    ),
    getFile: jest.fn(),
    putFile: jest.fn().mockResolvedValue(undefined),
    deleteFile: jest.fn().mockResolvedValue(undefined),
    getMyUniverses: jest.fn(),
    ping: jest.fn().mockResolvedValue(true),
  };
  return client as unknown as CoApiClient;
}

describe("CoFileSystemProvider", () => {
  const SLUG = "test-universe";

  describe("stat()", () => {
    it("returns Directory stat for root URI", async () => {
      const client = makeClient([]);
      const provider = new CoFileSystemProvider(client);
      const uri = Uri.parse(`co://${SLUG}/`) as unknown as import("vscode").Uri;

      const stat = await provider.stat(uri);
      expect(stat.type).toBe(FileType.Directory);
    });

    it("returns File stat for a known file", async () => {
      const client = makeClient([
        { path: "notes/foo.md", ctime: 100, mtime: 200, size: 42 },
      ]);
      const provider = new CoFileSystemProvider(client);
      const uri = Uri.parse(`co://${SLUG}/notes/foo.md`) as unknown as import("vscode").Uri;

      const stat = await provider.stat(uri);
      expect(stat.type).toBe(FileType.File);
      expect(stat.size).toBe(42);
      expect(stat.mtime).toBe(200);
    });

    it("returns Directory stat for an intermediate directory", async () => {
      const client = makeClient([
        { path: "notes/sub/deep.md" },
      ]);
      const provider = new CoFileSystemProvider(client);
      const uri = Uri.parse(`co://${SLUG}/notes`) as unknown as import("vscode").Uri;

      const stat = await provider.stat(uri);
      expect(stat.type).toBe(FileType.Directory);
    });

    it("throws FileNotFound for unknown file", async () => {
      const client = makeClient([]);
      const provider = new CoFileSystemProvider(client);
      const uri = Uri.parse(`co://${SLUG}/missing.md`) as unknown as import("vscode").Uri;

      await expect(provider.stat(uri)).rejects.toMatchObject({ code: "FileNotFound" });
    });
  });

  describe("readDirectory()", () => {
    it("lists root entries correctly", async () => {
      const client = makeClient([
        { path: "notes/a.md" },
        { path: "notes/b.md" },
        { path: "projects/CO/1.md" },
        { path: "index.md" },
      ]);
      const provider = new CoFileSystemProvider(client);
      const uri = Uri.parse(`co://${SLUG}/`) as unknown as import("vscode").Uri;

      const entries = await provider.readDirectory(uri);
      const names = entries.map(([name]) => name);
      expect(names).toContain("notes");
      expect(names).toContain("projects");
      expect(names).toContain("index.md");
      // notes and projects are directories
      const dirEntry = entries.find(([n]) => n === "notes");
      expect(dirEntry?.[1]).toBe(FileType.Directory);
      const fileEntry = entries.find(([n]) => n === "index.md");
      expect(fileEntry?.[1]).toBe(FileType.File);
    });

    it("lists entries inside a subdirectory", async () => {
      const client = makeClient([
        { path: "notes/a.md" },
        { path: "notes/sub/deep.md" },
        { path: "other.md" },
      ]);
      const provider = new CoFileSystemProvider(client);
      const uri = Uri.parse(`co://${SLUG}/notes`) as unknown as import("vscode").Uri;

      const entries = await provider.readDirectory(uri);
      const names = entries.map(([n]) => n);
      expect(names).toContain("a.md");
      expect(names).toContain("sub");
      expect(names).not.toContain("other.md");
    });
  });

  describe("writeFile()", () => {
    it("calls putFile with correct args and emits Changed event", async () => {
      const client = makeClient([]);
      const provider = new CoFileSystemProvider(client);
      const uri = Uri.parse(`co://${SLUG}/notes/new.md`) as unknown as import("vscode").Uri;

      const events: import("vscode").FileChangeEvent[][] = [];
      provider.onDidChangeFile((e) => events.push(e));

      await provider.writeFile(uri, Buffer.from("# Hello"), { create: true, overwrite: false });

      expect(client.putFile).toHaveBeenCalledWith(SLUG, "notes/new.md", "# Hello");
      expect(events).toHaveLength(1);
    });
  });

  describe("listFilesForSlug()", () => {
    it("returns all files for a slug", async () => {
      const client = makeClient([
        { path: "a.md" },
        { path: "b.md" },
      ]);
      const provider = new CoFileSystemProvider(client);

      const files = await provider.listFilesForSlug(SLUG);
      expect(files.map((f) => f.path)).toEqual(["a.md", "b.md"]);
    });
  });
});
