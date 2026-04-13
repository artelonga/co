// Unit tests — wikilink generation and resolution

import {
  mdLinksToWikilinks,
  wikilinksToMdLinks,
  generateWikilink,
  extractWikilinkIds,
  resolveParentRef,
  ensureParentDataviewField,
  parseParentDataviewField,
} from "../src/wikilinks";

// ---------------------------------------------------------------------------
// mdLinksToWikilinks
// ---------------------------------------------------------------------------
describe("mdLinksToWikilinks", () => {
  it("converts [Title](CO-21) to [[CO-21|Title]]", () => {
    const body = "See [Universe CRUD API](CO-21) for details.";
    expect(mdLinksToWikilinks(body)).toBe(
      "See [[CO-21|Universe CRUD API]] for details."
    );
  });

  it("converts multiple links", () => {
    const body = "[Task A](CO-5) and [Task B](CO-6)";
    expect(mdLinksToWikilinks(body)).toBe(
      "[[CO-5|Task A]] and [[CO-6|Task B]]"
    );
  });

  it("leaves non-CO links untouched", () => {
    const body = "[Google](https://google.com)";
    expect(mdLinksToWikilinks(body)).toBe("[Google](https://google.com)");
  });
});

// ---------------------------------------------------------------------------
// wikilinksToMdLinks
// ---------------------------------------------------------------------------
describe("wikilinksToMdLinks", () => {
  it("converts [[CO-21]] to [CO-21](CO-21)", () => {
    expect(wikilinksToMdLinks("[[CO-21]]")).toBe("[CO-21](CO-21)");
  });

  it("converts [[CO-21|Title]] to [Title](CO-21)", () => {
    expect(wikilinksToMdLinks("[[CO-21|Universe CRUD API]]")).toBe(
      "[Universe CRUD API](CO-21)"
    );
  });

  it("converts multiple wikilinks", () => {
    const body = "[[CO-5|Task A]] and [[CO-6|Task B]]";
    expect(wikilinksToMdLinks(body)).toBe("[Task A](CO-5) and [Task B](CO-6)");
  });

  it("leaves non-CO wikilinks untouched", () => {
    const body = "[[Some Note]]";
    expect(wikilinksToMdLinks(body)).toBe("[[Some Note]]");
  });
});

// ---------------------------------------------------------------------------
// Round-trip: mdLinks → wikilinks → mdLinks
// ---------------------------------------------------------------------------
describe("wikilink round-trip", () => {
  it("round-trips correctly", () => {
    const original = "[Universe CRUD API](CO-21)";
    const wikilink = mdLinksToWikilinks(original);
    const recovered = wikilinksToMdLinks(wikilink);
    expect(recovered).toBe(original);
  });
});

// ---------------------------------------------------------------------------
// generateWikilink
// ---------------------------------------------------------------------------
describe("generateWikilink", () => {
  it("generates [[CO-21|Title]] when title provided", () => {
    expect(generateWikilink(21, "Universe CRUD API")).toBe(
      "[[CO-21|Universe CRUD API]]"
    );
  });

  it("generates [[CO-21]] when no title", () => {
    expect(generateWikilink(21, undefined)).toBe("[[CO-21]]");
  });
});

// ---------------------------------------------------------------------------
// extractWikilinkIds
// ---------------------------------------------------------------------------
describe("extractWikilinkIds", () => {
  it("extracts IDs from body text", () => {
    const body = "See [[CO-21]] and [[CO-34|Obsidian plugin]].";
    expect(extractWikilinkIds(body)).toEqual([21, 34]);
  });

  it("returns empty array when no wikilinks", () => {
    expect(extractWikilinkIds("No wikilinks here.")).toEqual([]);
  });

  it("deduplicates when same id appears twice", () => {
    const body = "[[CO-21]] and again [[CO-21]]";
    // extractWikilinkIds returns all occurrences (not deduplicated)
    expect(extractWikilinkIds(body)).toEqual([21, 21]);
  });
});

// ---------------------------------------------------------------------------
// resolveParentRef
// ---------------------------------------------------------------------------
describe("resolveParentRef", () => {
  it("resolves [[CO-21]] → 21", () => {
    expect(resolveParentRef("[[CO-21]]")).toBe(21);
  });

  it("resolves [[CO-21|Title]] → 21", () => {
    expect(resolveParentRef("[[CO-21|Universe CRUD API]]")).toBe(21);
  });

  it("resolves plain 'CO-21' → 21", () => {
    expect(resolveParentRef("CO-21")).toBe(21);
  });

  it("returns null for unresolvable input", () => {
    expect(resolveParentRef("not a ref")).toBeNull();
    expect(resolveParentRef("[[Some Note]]")).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// ensureParentDataviewField
// ---------------------------------------------------------------------------
describe("ensureParentDataviewField", () => {
  it("inserts field at start when no blank line", () => {
    const body = "Single line body.";
    const result = ensureParentDataviewField(body, 20);
    expect(result).toContain("parent:: [[CO-20]]");
  });

  it("inserts field after first blank line", () => {
    const body = "First paragraph.\n\nSecond paragraph.";
    const result = ensureParentDataviewField(body, 20);
    expect(result).toContain("parent:: [[CO-20]]");
  });

  it("does not duplicate if field already present", () => {
    const body = "parent:: [[CO-20]]\n\nSome content.";
    const result = ensureParentDataviewField(body, 20);
    const count = (result.match(/parent:: \[\[CO-20\]\]/g) ?? []).length;
    expect(count).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// parseParentDataviewField
// ---------------------------------------------------------------------------
describe("parseParentDataviewField", () => {
  it("parses 'parent:: [[CO-20]]' from body", () => {
    const body = "parent:: [[CO-20]]\n\nSome content.";
    expect(parseParentDataviewField(body)).toBe(20);
  });

  it("parses 'parent:: [[CO-20|Title]]'", () => {
    const body = "parent:: [[CO-20|Epic Title]]\n\nSome content.";
    expect(parseParentDataviewField(body)).toBe(20);
  });

  it("returns null when field not present", () => {
    expect(parseParentDataviewField("No parent field.")).toBeNull();
  });
});
