// Wikilink support — CO ↔ Obsidian
//
// CO content references are rendered as [[CO-21|Title]] wikilinks in .md files.
// On import, [[CO-21]] is resolved back to a numeric reference.
//
// The plugin registers a link handler so clicking [[CO-21]] opens the synced file.

import { buildWikilinkWithTitle, parseWikilinkId } from "./frontmatter";

// ---------------------------------------------------------------------------
// Wikilink generation (CO → Obsidian body text)
// ---------------------------------------------------------------------------

/** Replace `[CO-21](...)` markdown links with `[[CO-21|Title]]` wikilinks */
export function mdLinksToWikilinks(body: string): string {
  // [Title](CO-21) or [Title](#CO-21)
  return body.replace(
    /\[([^\]]+)\]\((?:#?)(CO-(\d+))\)/g,
    (_match, title, _ref, id) => buildWikilinkWithTitle(parseInt(id, 10), title)
  );
}

/**
 * Generate a wikilink for a CO reference.
 * Given a path map (co-id → vault path), produce a proper [[file|title]] link
 * or fall back to [[CO-21]].
 */
export function generateWikilink(
  id: number,
  title: string | undefined,
  _pathMap?: Record<number, string>
): string {
  return buildWikilinkWithTitle(id, title);
}

// ---------------------------------------------------------------------------
// Wikilink resolution (Obsidian → CO)
// ---------------------------------------------------------------------------

/**
 * Extract all [[CO-XX]] references from markdown body.
 * Returns array of numeric IDs.
 */
export function extractWikilinkIds(body: string): number[] {
  const ids: number[] = [];
  const re = /\[\[CO-(\d+)(?:\|[^\]]+)?\]\]/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body)) !== null) {
    ids.push(parseInt(m[1], 10));
  }
  return ids;
}

/**
 * Replace [[CO-21]] or [[CO-21|Title]] wikilinks with markdown links [Title](CO-21).
 * Used when converting from Obsidian format back to CO storage format.
 */
export function wikilinksToMdLinks(body: string): string {
  return body.replace(
    /\[\[CO-(\d+)(?:\|([^\]]+))?\]\]/g,
    (_match, id, title) => {
      const label = title ?? `CO-${id}`;
      return `[${label}](CO-${id})`;
    }
  );
}

/**
 * Resolve a "parent" wikilink field into a numeric CO id.
 * Accepts "[[CO-21]]", "[[CO-21|Title]]", or plain "CO-21".
 */
export function resolveParentRef(ref: string): number | null {
  const fromWikilink = parseWikilinkId(ref);
  if (fromWikilink !== null) return fromWikilink;

  const plain = ref.match(/^CO-(\d+)$/);
  if (plain) return parseInt(plain[1], 10);

  return null;
}

// ---------------------------------------------------------------------------
// Inline Dataview fields
// ---------------------------------------------------------------------------

/**
 * Add "parent:: [[CO-21]]" inline field below the frontmatter in body text.
 * Only adds it if not already present.
 */
export function ensureParentDataviewField(body: string, parentId: number): string {
  const field = `parent:: [[CO-${parentId}]]`;
  if (body.includes(field)) return body;

  // Insert after first blank line (or at start)
  const firstBlankLine = body.indexOf("\n\n");
  if (firstBlankLine === -1) {
    return `${field}\n\n${body}`;
  }
  return `${body.slice(0, firstBlankLine + 2)}${field}\n\n${body.slice(firstBlankLine + 2)}`;
}

/**
 * Parse "parent:: [[CO-21]]" from body text, returning the numeric id or null.
 */
export function parseParentDataviewField(body: string): number | null {
  const m = body.match(/^parent::\s*\[\[CO-(\d+)(?:\|[^\]]+)?\]\]/m);
  if (!m) return null;
  return parseInt(m[1], 10);
}
