// Frontmatter mapping — CO ↔ Obsidian
//
// CO → Obsidian:
//   labels    → tags
//   created_at → created
//   updated_at → modified
//   parent: 21 → parent: "[[CO-21]]"   +  body line: "parent:: [[CO-21]]"
//   id, title, type, status, priority: preserved as-is
//   all other fields: preserved (round-trip safe)
//
// Obsidian → CO:
//   tags      → labels
//   created   → created_at
//   modified  → updated_at
//   parent: "[[CO-21]]" → parent: 21
//   aliases: preserved in metadata
//   all other fields: preserved (round-trip safe)

import type { CoFrontmatter, ObsidianFrontmatter } from "./types";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Parse "[[CO-21]]" or "[[CO-21|Some title]]" → 21, or null */
export function parseWikilinkId(wikilink: string): number | null {
  const m = wikilink.match(/^\[\[CO-(\d+)(?:\|[^\]]+)?\]\]$/);
  if (!m) return null;
  return parseInt(m[1], 10);
}

/** Build "[[CO-21]]" from a numeric id */
export function buildWikilink(id: number): string {
  return `[[CO-${id}]]`;
}

/** Build "[[CO-21|Title]]" from a numeric id and optional title */
export function buildWikilinkWithTitle(id: number, title?: string): string {
  if (title) return `[[CO-${id}|${title}]]`;
  return `[[CO-${id}]]`;
}

// ---------------------------------------------------------------------------
// CO → Obsidian
// ---------------------------------------------------------------------------

/**
 * Map CO frontmatter to Obsidian frontmatter.
 * Unknown fields are passed through unchanged (round-trip safe).
 */
export function coToObsidian(co: CoFrontmatter): ObsidianFrontmatter {
  const {
    labels,
    created_at,
    updated_at,
    parent,
    // extracted ↑
    ...rest
  } = co;

  const obs: ObsidianFrontmatter = { ...rest };

  if (labels !== undefined) {
    obs.tags = labels;
  }
  if (created_at !== undefined) {
    obs.created = created_at;
  }
  if (updated_at !== undefined) {
    obs.modified = updated_at;
  }
  if (parent !== undefined && parent !== null) {
    obs.parent = buildWikilink(parent);
  }

  return obs;
}

/**
 * Generate the Dataview inline field for the parent link.
 * Returns "" when there is no parent.
 */
export function parentDataviewLine(co: CoFrontmatter): string {
  if (co.parent == null) return "";
  return `parent:: ${buildWikilink(co.parent)}`;
}

// ---------------------------------------------------------------------------
// Obsidian → CO
// ---------------------------------------------------------------------------

/**
 * Map Obsidian frontmatter back to CO frontmatter.
 * Unknown fields are passed through unchanged (round-trip safe).
 */
export function obsidianToCo(obs: ObsidianFrontmatter): CoFrontmatter {
  const {
    tags,
    created,
    modified,
    parent,
    // extracted ↑
    ...rest
  } = obs;

  const co: CoFrontmatter = { ...rest };

  if (tags !== undefined) {
    co.labels = tags;
  }
  if (created !== undefined) {
    co.created_at = created;
  }
  if (modified !== undefined) {
    co.updated_at = modified;
  }
  if (parent !== undefined && parent !== null) {
    const id = parseWikilinkId(parent);
    if (id !== null) {
      co.parent = id;
    }
  }

  return co;
}

// ---------------------------------------------------------------------------
// Serialisation helpers
// ---------------------------------------------------------------------------

/**
 * Serialise a frontmatter object to YAML block (without leading/trailing ---).
 * Keeps field order: id, title, type, status, priority, tags/labels, dates, parent, aliases, rest.
 */
export function serialiseFrontmatter(
  fm: ObsidianFrontmatter | CoFrontmatter
): string {
  const ORDER = [
    "id",
    "title",
    "type",
    "status",
    "priority",
    "tags",
    "labels",
    "created",
    "created_at",
    "modified",
    "updated_at",
    "parent",
    "aliases",
  ];

  const lines: string[] = [];

  // Ordered fields first
  for (const key of ORDER) {
    if (!(key in fm)) continue;
    lines.push(serialiseField(key, fm[key as keyof typeof fm]));
  }

  // Remaining fields alphabetically
  const remaining = Object.keys(fm)
    .filter((k) => !ORDER.includes(k))
    .sort();
  for (const key of remaining) {
    lines.push(serialiseField(key, fm[key as keyof typeof fm]));
  }

  return lines.join("\n");
}

function serialiseField(key: string, value: unknown): string {
  if (Array.isArray(value)) {
    if (value.length === 0) return `${key}: []`;
    return `${key}:\n${value.map((v) => `  - ${String(v)}`).join("\n")}`;
  }
  if (value === null || value === undefined) return `${key}:`;
  if (typeof value === "string") {
    // Quote strings that contain ": " (key-value separator in YAML),
    // start with "[" or "{" (would be parsed as sequence/mapping),
    // or start with "#" (comment marker).
    // ISO 8601 timestamps (e.g. "2026-04-06T00:00:00Z") are safe unquoted.
    const needsQuote =
      value.includes(": ") ||
      value.startsWith("[") ||
      value.startsWith("{") ||
      value.startsWith("#") ||
      value.startsWith("*") ||
      value === "null" ||
      value === "true" ||
      value === "false";
    if (needsQuote) {
      return `${key}: "${value.replace(/"/g, '\\"')}"`;
    }
    return `${key}: ${value}`;
  }
  return `${key}: ${String(value)}`;
}

/**
 * Parse a YAML frontmatter block (content between --- markers) into a plain object.
 * Only handles simple scalar, list, and null values — good enough for CO frontmatter.
 */
export function parseFrontmatter(yaml: string): Record<string, unknown> {
  const result: Record<string, unknown> = {};
  const lines = yaml.split("\n");
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const keyMatch = line.match(/^([a-zA-Z_][a-zA-Z0-9_]*)\s*:(.*)/);
    if (!keyMatch) {
      i++;
      continue;
    }

    const key = keyMatch[1];
    const valueStr = keyMatch[2].trim();

    if (valueStr === "" || valueStr === null) {
      // Possible list following
      const items: string[] = [];
      i++;
      while (i < lines.length && lines[i].match(/^\s+-\s+(.*)/)) {
        const itemMatch = lines[i].match(/^\s+-\s+(.*)/);
        if (itemMatch) items.push(itemMatch[1].trim());
        i++;
      }
      result[key] = items.length > 0 ? items : null;
      continue;
    }

    // List on same line: key: [a, b]
    if (valueStr.startsWith("[") && valueStr.endsWith("]")) {
      const inner = valueStr.slice(1, -1).trim();
      result[key] =
        inner === "" ? [] : inner.split(",").map((s) => s.trim());
      i++;
      continue;
    }

    // Quoted string
    const quotedMatch = valueStr.match(/^"(.*)"$/);
    if (quotedMatch) {
      result[key] = quotedMatch[1].replace(/\\"/g, '"');
      i++;
      continue;
    }

    // Number
    if (/^\d+$/.test(valueStr)) {
      result[key] = parseInt(valueStr, 10);
      i++;
      continue;
    }

    // Boolean
    if (valueStr === "true") { result[key] = true; i++; continue; }
    if (valueStr === "false") { result[key] = false; i++; continue; }
    if (valueStr === "null" || valueStr === "~") { result[key] = null; i++; continue; }

    result[key] = valueStr;
    i++;
  }

  return result;
}

/**
 * Extract frontmatter block (between --- markers) from full markdown content.
 * Returns { frontmatter, body } where body is everything after the closing ---.
 */
export function extractFrontmatterBlock(content: string): {
  raw: string;
  body: string;
} {
  if (!content.startsWith("---")) {
    return { raw: "", body: content };
  }
  const secondSep = content.indexOf("\n---", 3);
  if (secondSep === -1) {
    return { raw: "", body: content };
  }
  const raw = content.slice(4, secondSep); // between the two ---
  const body = content.slice(secondSep + 4).replace(/^\n/, "");
  return { raw, body };
}

/**
 * Render full markdown file from Obsidian frontmatter + body.
 */
export function renderMarkdown(
  fm: ObsidianFrontmatter,
  body: string
): string {
  const fmStr = serialiseFrontmatter(fm);
  return `---\n${fmStr}\n---\n${body}`;
}
