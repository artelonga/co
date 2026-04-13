// Unit tests — frontmatter mapping (CO ↔ Obsidian round-trip)

import {
  coToObsidian,
  obsidianToCo,
  parseFrontmatter,
  serialiseFrontmatter,
  extractFrontmatterBlock,
  renderMarkdown,
  parseWikilinkId,
  buildWikilink,
  buildWikilinkWithTitle,
  parentDataviewLine,
} from "../src/frontmatter";
import type { CoFrontmatter, ObsidianFrontmatter } from "../src/types";

// ---------------------------------------------------------------------------
// parseWikilinkId
// ---------------------------------------------------------------------------
describe("parseWikilinkId", () => {
  it("parses [[CO-21]]", () => {
    expect(parseWikilinkId("[[CO-21]]")).toBe(21);
  });

  it("parses [[CO-21|Some title]]", () => {
    expect(parseWikilinkId("[[CO-21|Some title]]")).toBe(21);
  });

  it("returns null for invalid input", () => {
    expect(parseWikilinkId("not a wikilink")).toBeNull();
    expect(parseWikilinkId("[[21]]")).toBeNull();
    expect(parseWikilinkId("")).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// buildWikilink / buildWikilinkWithTitle
// ---------------------------------------------------------------------------
describe("buildWikilink", () => {
  it("builds [[CO-21]]", () => {
    expect(buildWikilink(21)).toBe("[[CO-21]]");
  });

  it("builds [[CO-21|Title]] when title provided", () => {
    expect(buildWikilinkWithTitle(21, "Universe CRUD API")).toBe(
      "[[CO-21|Universe CRUD API]]"
    );
  });

  it("builds [[CO-21]] when title is undefined", () => {
    expect(buildWikilinkWithTitle(21, undefined)).toBe("[[CO-21]]");
  });
});

// ---------------------------------------------------------------------------
// coToObsidian
// ---------------------------------------------------------------------------
describe("coToObsidian", () => {
  it("maps labels → tags", () => {
    const co: CoFrontmatter = { labels: ["rust", "backend"] };
    const obs = coToObsidian(co);
    expect(obs.tags).toEqual(["rust", "backend"]);
    expect("labels" in obs).toBe(false);
  });

  it("maps created_at → created", () => {
    const co: CoFrontmatter = { created_at: "2026-04-06T00:00:00Z" };
    const obs = coToObsidian(co);
    expect(obs.created).toBe("2026-04-06T00:00:00Z");
    expect("created_at" in obs).toBe(false);
  });

  it("maps updated_at → modified", () => {
    const co: CoFrontmatter = { updated_at: "2026-04-06T00:00:00Z" };
    const obs = coToObsidian(co);
    expect(obs.modified).toBe("2026-04-06T00:00:00Z");
    expect("updated_at" in obs).toBe(false);
  });

  it("maps parent: 21 → parent: '[[CO-21]]'", () => {
    const co: CoFrontmatter = { parent: 21 };
    const obs = coToObsidian(co);
    expect(obs.parent).toBe("[[CO-21]]");
  });

  it("preserves unknown fields round-trip safe", () => {
    const co: CoFrontmatter = {
      id: 5,
      title: "My Task",
      status: "in-progress",
      custom_field: "value",
    };
    const obs = coToObsidian(co);
    expect(obs.id).toBe(5);
    expect(obs.title).toBe("My Task");
    expect(obs.status).toBe("in-progress");
    expect(obs.custom_field).toBe("value");
  });

  it("preserves aliases", () => {
    const co: CoFrontmatter = { aliases: ["alt-name"] };
    const obs = coToObsidian(co);
    expect(obs.aliases).toEqual(["alt-name"]);
  });
});

// ---------------------------------------------------------------------------
// obsidianToCo
// ---------------------------------------------------------------------------
describe("obsidianToCo", () => {
  it("maps tags → labels", () => {
    const obs: ObsidianFrontmatter = { tags: ["rust", "backend"] };
    const co = obsidianToCo(obs);
    expect(co.labels).toEqual(["rust", "backend"]);
    expect("tags" in co).toBe(false);
  });

  it("maps created → created_at", () => {
    const obs: ObsidianFrontmatter = { created: "2026-04-06T00:00:00Z" };
    const co = obsidianToCo(obs);
    expect(co.created_at).toBe("2026-04-06T00:00:00Z");
    expect("created" in co).toBe(false);
  });

  it("maps modified → updated_at", () => {
    const obs: ObsidianFrontmatter = { modified: "2026-04-06T00:00:00Z" };
    const co = obsidianToCo(obs);
    expect(co.updated_at).toBe("2026-04-06T00:00:00Z");
    expect("modified" in co).toBe(false);
  });

  it("maps parent: '[[CO-21]]' → parent: 21", () => {
    const obs: ObsidianFrontmatter = { parent: "[[CO-21]]" };
    const co = obsidianToCo(obs);
    expect(co.parent).toBe(21);
  });

  it("maps parent: '[[CO-21|Title]]' → parent: 21", () => {
    const obs: ObsidianFrontmatter = { parent: "[[CO-21|Universe CRUD API]]" };
    const co = obsidianToCo(obs);
    expect(co.parent).toBe(21);
  });

  it("preserves unknown fields round-trip safe", () => {
    const obs: ObsidianFrontmatter = {
      id: 5,
      title: "My Task",
      custom_field: "value",
    };
    const co = obsidianToCo(obs);
    expect(co.id).toBe(5);
    expect(co.title).toBe("My Task");
    expect(co.custom_field).toBe("value");
  });

  it("preserves aliases in metadata", () => {
    const obs: ObsidianFrontmatter = { aliases: ["alt-name"] };
    const co = obsidianToCo(obs);
    expect(co.aliases).toEqual(["alt-name"]);
  });
});

// ---------------------------------------------------------------------------
// Full round-trip: CO → Obsidian → CO
// ---------------------------------------------------------------------------
describe("round-trip CO → Obsidian → CO", () => {
  it("round-trips a complete CO frontmatter", () => {
    const original: CoFrontmatter = {
      id: 34,
      title: "Obsidian plugin",
      type: "task",
      status: "in-progress",
      priority: "high",
      labels: ["obsidian", "plugin"],
      created_at: "2026-04-06T00:00:00Z",
      updated_at: "2026-04-06T00:00:00Z",
      parent: 20,
      aliases: ["co-34"],
      extra_field: "preserved",
    };

    const obs = coToObsidian(original);
    const recovered = obsidianToCo(obs);

    expect(recovered.id).toBe(34);
    expect(recovered.title).toBe("Obsidian plugin");
    expect(recovered.labels).toEqual(["obsidian", "plugin"]);
    expect(recovered.created_at).toBe("2026-04-06T00:00:00Z");
    expect(recovered.parent).toBe(20);
    expect(recovered.aliases).toEqual(["co-34"]);
    expect(recovered.extra_field).toBe("preserved");
  });
});

// ---------------------------------------------------------------------------
// parseFrontmatter
// ---------------------------------------------------------------------------
describe("parseFrontmatter", () => {
  it("parses simple scalars", () => {
    const yaml = `id: 34\ntitle: My Task\nstatus: done`;
    const fm = parseFrontmatter(yaml);
    expect(fm.id).toBe(34);
    expect(fm.title).toBe("My Task");
    expect(fm.status).toBe("done");
  });

  it("parses list values", () => {
    const yaml = `tags:\n  - rust\n  - backend`;
    const fm = parseFrontmatter(yaml);
    expect(fm.tags).toEqual(["rust", "backend"]);
  });

  it("parses inline list", () => {
    const yaml = `tags: [rust, backend]`;
    const fm = parseFrontmatter(yaml);
    expect(fm.tags).toEqual(["rust", "backend"]);
  });

  it("parses booleans", () => {
    const yaml = `flag: true\nother: false`;
    const fm = parseFrontmatter(yaml);
    expect(fm.flag).toBe(true);
    expect(fm.other).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// serialiseFrontmatter
// ---------------------------------------------------------------------------
describe("serialiseFrontmatter", () => {
  it("serialises key-value pairs", () => {
    const fm: ObsidianFrontmatter = { id: 5, title: "Hello" };
    const yaml = serialiseFrontmatter(fm);
    expect(yaml).toContain("id: 5");
    expect(yaml).toContain("title: Hello");
  });

  it("serialises arrays as YAML lists", () => {
    const fm: ObsidianFrontmatter = { tags: ["rust", "backend"] };
    const yaml = serialiseFrontmatter(fm);
    expect(yaml).toContain("tags:");
    expect(yaml).toContain("  - rust");
    expect(yaml).toContain("  - backend");
  });

  it("quotes strings with colons", () => {
    const fm: ObsidianFrontmatter = { title: "Hello: World" };
    const yaml = serialiseFrontmatter(fm);
    expect(yaml).toContain('"Hello: World"');
  });
});

// ---------------------------------------------------------------------------
// extractFrontmatterBlock
// ---------------------------------------------------------------------------
describe("extractFrontmatterBlock", () => {
  it("extracts frontmatter and body", () => {
    const content = `---\nid: 5\ntitle: Test\n---\nBody text here.`;
    const { raw, body } = extractFrontmatterBlock(content);
    expect(raw.trim()).toBe("id: 5\ntitle: Test");
    expect(body.trim()).toBe("Body text here.");
  });

  it("returns empty raw when no frontmatter", () => {
    const content = "No frontmatter here.";
    const { raw, body } = extractFrontmatterBlock(content);
    expect(raw).toBe("");
    expect(body).toBe("No frontmatter here.");
  });
});

// ---------------------------------------------------------------------------
// parentDataviewLine
// ---------------------------------------------------------------------------
describe("parentDataviewLine", () => {
  it("generates parent:: field for a CO frontmatter with parent", () => {
    const co: CoFrontmatter = { parent: 21 };
    expect(parentDataviewLine(co)).toBe("parent:: [[CO-21]]");
  });

  it("returns empty string when no parent", () => {
    const co: CoFrontmatter = {};
    expect(parentDataviewLine(co)).toBe("");
  });
});

// ---------------------------------------------------------------------------
// renderMarkdown
// ---------------------------------------------------------------------------
describe("renderMarkdown", () => {
  it("wraps frontmatter in --- delimiters", () => {
    const fm: ObsidianFrontmatter = { title: "Test" };
    const md = renderMarkdown(fm, "Body text.");
    expect(md).toMatch(/^---\n/);
    expect(md).toContain("title: Test");
    expect(md).toContain("---\nBody text.");
  });
});
