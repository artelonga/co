/**
 * CO-382 / CO-443: DoD (Definition of Done) verification for a CO task spec.
 *
 * Reads work/co/CO-N.md, parses the ## Acceptance checklist, and scores each
 * item as ✅/⚠️ by one of two strategies:
 *
 *   1. Test-name matching (CO-382): derive a pattern from the item text and
 *      look for a matching test — Playwright `test('…')` in e2e specs AND
 *      Rust `#[test]` / `#[tokio::test]` functions (CO-443).
 *   2. Structural proofs (CO-443): an item can carry one or more proof
 *      directives (grep / grep-absent / rust-test / e2e-test / file) declared
 *      in the spec frontmatter `dod_checks:` map. This lets refactor/structural
 *      tasks — whose acceptance is "trait X moved to core", "game-core is
 *      axum-free", "no raw SQL in handlers" — score without an e2e test.
 *
 * Usage (from co-web/):
 *   node --import tsx scripts/dod/verify.ts --spec CO-382
 *   node --import tsx scripts/dod/verify.ts --spec CO-382 --generate-stubs
 *   node --import tsx scripts/dod/verify.ts --spec CO-382 --post-comment
 *
 * Env for PR comments:
 *   GH_TOKEN — GitHub token with `issues:write`
 *   PR_NUMBER — pull request number
 *   REPO      — owner/repo string
 *
 * See docs/scrum/dod-proof-checks.md for how to write proof-verifiable AC.
 */

import {
  readFileSync,
  writeFileSync,
  readdirSync,
  statSync,
  mkdirSync,
  existsSync,
  realpathSync,
} from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { argv, env } from 'node:process';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, '../../..');
const WORK_CO_DIR = join(REPO_ROOT, 'work/co');
const E2E_DIR = join(REPO_ROOT, 'co-web/e2e');
const E2E_GEN_DIR = join(REPO_ROOT, 'co-web/e2e-generated');
const DOD_OUTPUT_DIR = join(REPO_ROOT, 'docs/scrum/dod');

/** Roots scanned for Rust tests and for unscoped `grep:` proofs. */
const DEFAULT_RUST_ROOTS = [
  'core/src',
  'core/tests',
  'co-web/src',
  'co-web/tests',
  'game-core/src',
  'game-core/tests',
  'co-cli/src',
  'co-cli/tests',
];

/** Extensions read when a proof scope is a directory. */
const SCOPE_EXTS = ['.rs', '.toml', '.ts', '.tsx', '.js', '.sql', '.md', '.yaml', '.yml'];

const TIMESTAMP = new Date().toISOString();

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface AcceptanceItem {
  text: string;
  checked: boolean;
}

/** A `<match substring> => <proof>[ && <proof>]` entry from frontmatter. */
export interface DodCheck {
  match: string;
  proofs: string[];
}

interface DodItem {
  text: string;
  pattern: string;
  matched_file: string | null;
  matched_test: string | null;
  is_stub: boolean;
  /** Proof directives evaluated for this item (empty → test-name matching). */
  proofs: string[];
  /** Human-readable evidence (matched test, or per-proof results). */
  evidence: string | null;
  // 'pending' = stub, no matching test, or an unmet structural proof —
  // advisory, never blocks. 'fail' is reserved for a matched real test that
  // fails when executed; existence-only matching (v1) cannot produce it.
  status: 'pass' | 'pending' | 'fail';
}

interface DodReport {
  co_id: string;
  spec_title: string;
  timestamp: string;
  items: DodItem[];
  total: number;
  passed: number;
  pending: number;
  failed: number;
  /** Only real matched-test failures block CI and the release gate. */
  blocking_failures: number;
  dod_pct: number;
}

// ---------------------------------------------------------------------------
// Spec parsing
// ---------------------------------------------------------------------------

/**
 * Parse the `dod_checks:` frontmatter list. Each entry is a quoted string of
 * the form `"<match substring> => <proof>[ && <proof>]"`:
 *
 *   dod_checks:
 *     - "trait moved to core => grep:pub trait Universo@core/src/universo.rs"
 *     - "game-core axum-free => grep-absent:^axum@game-core/Cargo.toml"
 *
 * The match substring is matched (case-insensitive, contains) against each
 * acceptance item; the proofs become that item's evidence.
 */
export function parseDodChecks(content: string): DodCheck[] {
  const fmMatch = content.match(/^---\n([\s\S]*?)\n---/);
  const fm = fmMatch ? fmMatch[1] : content;
  const lines = fm.split('\n');
  const checks: DodCheck[] = [];
  let inBlock = false;
  for (const line of lines) {
    if (/^dod_checks:\s*$/.test(line)) {
      inBlock = true;
      continue;
    }
    if (inBlock) {
      // A non-indented, non-list line ends the block.
      if (!/^\s+-\s+/.test(line)) {
        if (line.trim() === '') continue;
        break;
      }
      const raw = line.replace(/^\s+-\s+/, '').trim().replace(/^["']|["']$/g, '');
      const arrow = raw.indexOf('=>');
      if (arrow < 0) continue;
      const match = raw.slice(0, arrow).trim();
      const proofs = raw
        .slice(arrow + 2)
        .split('&&')
        .map(p => p.trim())
        .filter(Boolean);
      if (match && proofs.length) checks.push({ match, proofs });
    }
  }
  return checks;
}

export function parseSpecFile(coId: string): {
  title: string;
  items: AcceptanceItem[];
  dodChecks: DodCheck[];
} {
  const specPath = join(WORK_CO_DIR, `${coId}.md`);
  if (!existsSync(specPath)) {
    throw new Error(`Spec not found: ${specPath}`);
  }
  const content = readFileSync(specPath, 'utf-8');

  // Extract title from frontmatter
  const titleMatch = content.match(/^title:\s*"?([^"\n]+)"?/m);
  const title = titleMatch ? titleMatch[1].trim() : coId;

  const dodChecks = parseDodChecks(content);

  // Find the ## Acceptance section and extract its checklist items. Done
  // line-by-line (not one regex) so a multi-line section is captured in full
  // up to the next ## heading — an earlier regex used `\z`, which JS treats
  // as a literal 'z' and truncated the section at the first such character.
  const allLines = content.split('\n');
  let inAcceptance = false;
  const sectionLines: string[] = [];
  for (const line of allLines) {
    if (/^##\s+(?:Acceptance|Crit[ée]rios)\b/.test(line)) {
      inAcceptance = true;
      continue;
    }
    if (inAcceptance && /^##\s/.test(line)) break; // next section ends it
    if (inAcceptance) sectionLines.push(line);
  }
  // Fallback: specs without an Acceptance section (pt-style Escopo specs)
  // still get their checkboxes harvested from the whole document, so the
  // DoD report is never a trivial 0/0 = 100%.
  const section = sectionLines.length > 0 ? sectionLines.join('\n') : content;
  const items: AcceptanceItem[] = [];
  // Acceptance items wrap across several indented lines. Join each wrapped
  // continuation into its bullet so the full criterion text is available for
  // proof matching (dod_checks) and pattern derivation — not just line one.
  let current: AcceptanceItem | null = null;
  for (const line of section.split('\n')) {
    const unchecked = line.match(/^[-*]\s+\[\s\]\s+(.+)/);
    const checked = line.match(/^[-*]\s+\[x\]\s+(.+)/i);
    if (unchecked || checked) {
      if (current) items.push(current);
      current = { text: (unchecked ?? checked)![1].trim(), checked: Boolean(checked) };
    } else if (current && /^\s+\S/.test(line)) {
      current.text += ' ' + line.trim();
    } else if (current) {
      items.push(current);
      current = null;
    }
  }
  if (current) items.push(current);

  return { title, items, dodChecks };
}

/** Collect proof directives whose match substring is present in the item. */
export function matchProofsForItem(itemText: string, checks: DodCheck[]): string[] {
  const lower = itemText.toLowerCase();
  const proofs: string[] = [];
  for (const check of checks) {
    if (lower.includes(check.match.toLowerCase())) proofs.push(...check.proofs);
  }
  return proofs;
}

// ---------------------------------------------------------------------------
// Pattern derivation
// ---------------------------------------------------------------------------

const STOPWORDS = new Set([
  'the', 'and', 'for', 'that', 'with', 'this', 'from', 'have', 'when',
  'each', 'item', 'spec', 'test', 'must', 'does', 'not', 'all', 'any',
  'are', 'into', 'via', 'per', 'its', 'has', 'can', 'new', 'get',
]);

/**
 * Derive a regex pattern from a DoD text item.
 * Strategy: extract hyphenated identifiers + meaningful keywords.
 * Example: "btn-compor opens compose modal" → "btn.compor|compose|modal"
 */
export function derivePattern(text: string): string {
  const lower = text.toLowerCase();

  // 1. Extract hyphenated identifiers (e.g., btn-compor, e2e-staging)
  const kebabIds = lower.match(/[a-z][a-z0-9]*(?:-[a-z0-9]+)+/g) ?? [];

  // 2. Extract path-like tokens (e.g., /u/comunicacao/sala)
  const paths = lower.match(/\/[a-z][a-z0-9/.-]+/g) ?? [];

  // 3. Extract meaningful words (>4 chars, not stopwords)
  const words = lower
    .replace(/[^a-z0-9\s-]/g, ' ')
    .split(/\s+/)
    .filter(w => w.length > 4 && !STOPWORDS.has(w))
    .slice(0, 4);

  const terms = [...new Set([...kebabIds, ...paths.map(p => p.slice(1)), ...words])];
  if (terms.length === 0) {
    // Fallback: first 3 tokens of any length
    const fallback = lower.split(/\s+/).slice(0, 3).join('.*');
    return fallback;
  }

  // Escape special regex chars except '*', then build alternation
  return terms
    .map(t => t.replace(/[+?^${}()|[\]\\]/g, '\\$&').replace(/-/g, '[- _]'))
    .join('|');
}

// ---------------------------------------------------------------------------
// File walking + proof engine (CO-443)
// ---------------------------------------------------------------------------

const IGNORE_DIRS = new Set(['node_modules', 'target', '.git', 'dist', '.svelte-kit']);

/** Recursively list files under `absRoot` matching one of `exts`. */
function walkFiles(absRoot: string, exts: string[]): string[] {
  if (!existsSync(absRoot)) return [];
  let st;
  try {
    st = statSync(absRoot);
  } catch {
    return [];
  }
  if (st.isFile()) return [absRoot];
  const out: string[] = [];
  for (const name of readdirSync(absRoot)) {
    if (IGNORE_DIRS.has(name)) continue;
    const p = join(absRoot, name);
    let s;
    try {
      s = statSync(p);
    } catch {
      continue;
    }
    if (s.isDirectory()) out.push(...walkFiles(p, exts));
    else if (exts.some(e => name.endsWith(e))) out.push(p);
  }
  return out;
}

/** Resolve the file set a proof scope refers to (relative to repo root). */
function filesForScope(scope: string | undefined): string[] {
  if (scope) return walkFiles(join(REPO_ROOT, scope), SCOPE_EXTS);
  return DEFAULT_RUST_ROOTS.flatMap(r => walkFiles(join(REPO_ROOT, r), ['.rs']));
}

/** Count lines matching `pattern` across the files in `scope`. */
function grepCount(pattern: string, scope: string | undefined): number {
  let re: RegExp;
  try {
    re = new RegExp(pattern);
  } catch {
    return 0;
  }
  let count = 0;
  for (const file of filesForScope(scope)) {
    let content: string;
    try {
      content = readFileSync(file, 'utf-8');
    } catch {
      continue;
    }
    for (const line of content.split('\n')) {
      if (re.test(line)) count++;
    }
  }
  return count;
}

export interface ProofResult {
  ok: boolean;
  detail: string;
}

/**
 * Evaluate one proof directive. Supported kinds:
 *   grep:<regex>[@<scope>]          ≥1 matching line  (structural assertion)
 *   grep-absent:<regex>[@<scope>]   0 matching lines  (e.g. axum-free)
 *   rust-test:<fn_name>             a #[test]/#[tokio::test] fn of that name
 *   e2e-test:<regex>                a Playwright test whose name matches
 *   file:<path>                     the path exists
 *
 * `@<scope>` restricts the search to a file or directory (relative to repo
 * root). Without it, `grep` scans the Rust source roots. All proofs are pure
 * filesystem reads — no build, no network — so the check is deterministic.
 */
export function evaluateProof(proof: string): ProofResult {
  const colon = proof.indexOf(':');
  if (colon < 0) return { ok: false, detail: `malformed proof: ${proof}` };
  const kind = proof.slice(0, colon).trim();
  const rest = proof.slice(colon + 1).trim();
  const at = rest.indexOf('@');
  const arg = at >= 0 ? rest.slice(0, at).trim() : rest;
  const scope = at >= 0 ? rest.slice(at + 1).trim() : undefined;

  switch (kind) {
    case 'grep': {
      const n = grepCount(arg, scope);
      return { ok: n > 0, detail: `grep "${arg}"${scope ? ` @${scope}` : ''} → ${n}` };
    }
    case 'grep-absent': {
      const n = grepCount(arg, scope);
      return { ok: n === 0, detail: `grep-absent "${arg}"${scope ? ` @${scope}` : ''} → ${n}` };
    }
    case 'rust-test': {
      const found = findRustTest(arg);
      return {
        ok: found != null,
        detail: found ? `rust-test ${arg} → ${found.file}` : `rust-test ${arg} → not found`,
      };
    }
    case 'e2e-test': {
      const m = findMatchingE2eTest(arg);
      return {
        ok: m != null,
        detail: m ? `e2e-test → ${m.testName}` : `e2e-test /${arg}/ → not found`,
      };
    }
    case 'file': {
      const ok = existsSync(join(REPO_ROOT, arg));
      return { ok, detail: `file ${arg} → ${ok ? 'exists' : 'missing'}` };
    }
    default:
      return { ok: false, detail: `unknown proof kind: ${kind}` };
  }
}

// ---------------------------------------------------------------------------
// Test discovery — e2e specs + Rust tests
// ---------------------------------------------------------------------------

interface MatchedTest {
  file: string;
  testName: string;
  isStub: boolean;
}

function listSpecFiles(dir: string): string[] {
  if (!existsSync(dir)) return [];
  try {
    return readdirSync(dir, { recursive: true } as never)
      .filter((f: unknown) => typeof f === 'string' && f.endsWith('.spec.ts'))
      .map((f: string) => join(dir, f));
  } catch {
    return [];
  }
}

/**
 * Extract test() call names from a spec file content.
 * Handles: test('name', ...), test("name", ...), test(`name`, ...)
 */
function extractTestNames(content: string): Array<{ name: string; lineIdx: number }> {
  const results: Array<{ name: string; lineIdx: number }> = [];
  const lines = content.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^\s*test\s*\(\s*(['"`])([\s\S]*?)\1/);
    if (m) {
      results.push({ name: m[2], lineIdx: i });
    }
  }
  return results;
}

/**
 * Extract Rust test functions from a source file: a `fn name(` preceded
 * (within a few lines) by a `#[test]`, `#[tokio::test]`, or `#[…::test]`
 * attribute. (CO-443)
 */
export function extractRustTests(content: string): Array<{ name: string; lineIdx: number }> {
  const results: Array<{ name: string; lineIdx: number }> = [];
  const lines = content.split('\n');
  const isTestAttr = (l: string) =>
    /^\s*#\[\s*(?:tokio::|async_std::|actix_rt::|rstest)?test\b/.test(l) ||
    /^\s*#\[test\]/.test(l);
  for (let i = 0; i < lines.length; i++) {
    const m = lines[i].match(/^\s*(?:pub\s+)?(?:async\s+)?fn\s+([a-z_][a-z0-9_]*)\s*\(/i);
    if (!m) continue;
    // Look back over the attribute block immediately above the fn.
    let attrTest = false;
    for (let j = i - 1; j >= 0 && j >= i - 6; j--) {
      const prev = lines[j].trim();
      if (prev === '') continue;
      if (isTestAttr(lines[j])) {
        attrTest = true;
        break;
      }
      // Stop once we leave the contiguous attribute/doc block above the fn.
      if (!prev.startsWith('#[') && !prev.startsWith('//') && !prev.startsWith('///')) break;
    }
    if (attrTest) results.push({ name: m[1], lineIdx: i });
  }
  return results;
}

/** All Rust test functions across the source roots, cached per process. */
let rustTestCache: Array<{ name: string; file: string }> | null = null;
function allRustTests(): Array<{ name: string; file: string }> {
  if (rustTestCache) return rustTestCache;
  const out: Array<{ name: string; file: string }> = [];
  const files = DEFAULT_RUST_ROOTS.flatMap(r => walkFiles(join(REPO_ROOT, r), ['.rs']));
  for (const file of files) {
    let content: string;
    try {
      content = readFileSync(file, 'utf-8');
    } catch {
      continue;
    }
    for (const t of extractRustTests(content)) {
      out.push({ name: t.name, file: file.replace(REPO_ROOT + '/', '') });
    }
  }
  rustTestCache = out;
  return out;
}

function findRustTest(name: string): { file: string } | null {
  const hit = allRustTests().find(t => t.name === name);
  return hit ? { file: hit.file } : null;
}

/**
 * Check if a test at lineIdx has test.fixme() nearby (within 5 lines).
 */
function isFixmeTest(lines: string[], lineIdx: number): boolean {
  for (let i = lineIdx; i < Math.min(lineIdx + 8, lines.length); i++) {
    if (lines[i].includes('test.fixme()') || lines[i].includes('test.fixme(')) {
      return true;
    }
  }
  return false;
}

function findMatchingE2eTest(pattern: string): MatchedTest | null {
  let regex: RegExp;
  try {
    regex = new RegExp(pattern, 'i');
  } catch {
    return null;
  }
  const files = [E2E_DIR, E2E_GEN_DIR].flatMap(listSpecFiles);
  for (const file of files) {
    const content = readFileSync(file, 'utf-8');
    const lines = content.split('\n');
    for (const { name, lineIdx } of extractTestNames(content)) {
      if (regex.test(name)) {
        return {
          file: file.replace(REPO_ROOT + '/', ''),
          testName: name,
          isStub: isFixmeTest(lines, lineIdx),
        };
      }
    }
  }
  return null;
}

/**
 * Find a test matching `pattern`, preferring real e2e specs, then Rust tests.
 * Rust test names use snake_case, so the derived `word|word` alternation is
 * tested against the function name directly. (CO-443 generalised the matcher
 * to recognise Rust tests, not only e2e specs.)
 */
function findMatchingTest(pattern: string): MatchedTest | null {
  const e2e = findMatchingE2eTest(pattern);
  if (e2e) return e2e;

  let regex: RegExp;
  try {
    regex = new RegExp(pattern, 'i');
  } catch {
    return null;
  }
  for (const t of allRustTests()) {
    if (regex.test(t.name)) {
      return { file: t.file, testName: t.name, isStub: false };
    }
  }
  return null;
}

// ---------------------------------------------------------------------------
// Stub generation
// ---------------------------------------------------------------------------

function generateStubs(coId: string, items: AcceptanceItem[], title: string): void {
  mkdirSync(E2E_GEN_DIR, { recursive: true });
  const slug = coId.toLowerCase().replace('co-', 'co-');
  const outPath = join(E2E_GEN_DIR, `${slug}-dod.spec.ts`);

  const stubs = items.map(item => {
    const label = item.text.replace(/[`']/g, '').slice(0, 80);
    return `  test('[acceptance] ${label}', async ({ page }) => {\n    // TODO: implement\n    test.fixme();\n  });`;
  });

  const content = [
    `// Generated by scripts/dod/verify.ts — ${coId} DoD stubs`,
    `// Fill in each test and remove test.fixme() to satisfy the acceptance item.`,
    `import { test, expect } from '@playwright/test';`,
    ``,
    `test.describe('${coId} DoD — ${title.slice(0, 60)}', () => {`,
    ...stubs,
    `});`,
    ``,
  ].join('\n');

  writeFileSync(outPath, content);
  console.log(`Generated stubs: ${outPath}`);
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

export function buildReport(
  coId: string,
  title: string,
  items: AcceptanceItem[],
  dodChecks: DodCheck[] = [],
): DodReport {
  const dodItems: DodItem[] = items.map(item => {
    const proofs = matchProofsForItem(item.text, dodChecks);

    // Strategy 1: explicit structural proofs (CO-443).
    if (proofs.length > 0) {
      const results = proofs.map(p => ({ proof: p, ...evaluateProof(p) }));
      const allOk = results.every(r => r.ok);
      return {
        text: item.text,
        pattern: proofs.join(' && '),
        matched_file: null,
        matched_test: null,
        is_stub: false,
        proofs,
        evidence: results.map(r => `${r.ok ? '✓' : '✗'} ${r.detail}`).join('; '),
        status: allOk ? 'pass' : 'pending',
      };
    }

    // Strategy 2: test-name matching (e2e specs + Rust tests).
    const pattern = derivePattern(item.text);
    const match = findMatchingTest(pattern);
    const status: DodItem['status'] = !match || match.isStub ? 'pending' : 'pass';
    return {
      text: item.text,
      pattern,
      matched_file: match?.file ?? null,
      matched_test: match?.testName ?? null,
      is_stub: match?.isStub ?? false,
      proofs: [],
      evidence: match ? `${match.testName}${match.isStub ? ' (stub)' : ''}` : null,
      status,
    };
  });

  const passed = dodItems.filter(i => i.status === 'pass').length;
  const pending = dodItems.filter(i => i.status === 'pending').length;
  const failed = dodItems.filter(i => i.status === 'fail').length;
  const total = dodItems.length;

  return {
    co_id: coId,
    spec_title: title,
    timestamp: TIMESTAMP,
    items: dodItems,
    total,
    passed,
    pending,
    failed,
    blocking_failures: failed,
    dod_pct: total === 0 ? 100 : Math.round((passed / total) * 100),
  };
}

// ---------------------------------------------------------------------------
// PR comment
// ---------------------------------------------------------------------------

async function postPrComment(report: DodReport): Promise<void> {
  const token = env.GH_TOKEN;
  const prNumber = env.PR_NUMBER;
  const repo = env.REPO;

  if (!token || !prNumber || !repo) {
    console.warn('Skipping PR comment: GH_TOKEN, PR_NUMBER, REPO not all set');
    return;
  }

  const rows = report.items.map(item => {
    const icon = item.status === 'pass' ? '✅' : item.status === 'pending' ? '⚠️' : '❌';
    const evidence = item.evidence ? `\`${item.evidence.slice(0, 60)}\`` : '_no evidence_';
    return `| ${icon} | ${item.text.slice(0, 80)} | ${evidence} |`;
  });

  const body = [
    `## 🛠️ DoD Verification — ${report.co_id}`,
    ``,
    `**${report.dod_pct}% complete** (${report.passed}/${report.total} acceptance items passing)`,
    ``,
    `| | Acceptance Item | Evidence |`,
    `|---|---|---|`,
    ...rows,
    ``,
    report.blocking_failures > 0
      ? `> ❌ **Merge blocked**: ${report.blocking_failures} matched test(s) failing.`
      : report.pending > 0
        ? `> ⚠️ Advisory: ${report.pending} acceptance item(s) pending real tests/proofs. Merge not blocked.`
        : `> ✅ All acceptance items verified — merge allowed.`,
    ``,
    `_Generated by \`scripts/dod/verify.ts\` — ${report.timestamp}_`,
  ].join('\n');

  const url = `https://api.github.com/repos/${repo}/issues/${prNumber}/comments`;
  const resp = await fetch(url, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${token}`,
      'Content-Type': 'application/json',
      'User-Agent': 'co-dod-verify/1.0',
    },
    body: JSON.stringify({ body }),
  });

  if (!resp.ok) {
    const text = await resp.text();
    console.warn(`PR comment failed (${resp.status}): ${text}`);
  } else {
    console.log(`PR comment posted to ${repo}#${prNumber}`);
  }
}

// ---------------------------------------------------------------------------
// Main (CLI)
// ---------------------------------------------------------------------------

async function main(): Promise<void> {
  const args = argv.slice(2);
  const getArg = (flag: string): string | undefined => {
    const i = args.indexOf(flag);
    return i >= 0 ? args[i + 1] : undefined;
  };
  const FLAG_GENERATE_STUBS = args.includes('--generate-stubs');
  const FLAG_POST_COMMENT = args.includes('--post-comment');
  const specArg = getArg('--spec');
  const outputArg = getArg('--output');

  if (!specArg) {
    console.error('Usage: verify.ts --spec CO-NNN [--generate-stubs] [--post-comment] [--output path]');
    process.exit(1);
  }

  const CO_ID = specArg.toUpperCase().startsWith('CO-') ? specArg.toUpperCase() : `CO-${specArg}`;
  const { title, items, dodChecks } = parseSpecFile(CO_ID);

  if (items.length === 0) {
    console.warn(`No acceptance items found in ${CO_ID}.md — check ## Acceptance section`);
  }

  if (FLAG_GENERATE_STUBS) {
    generateStubs(CO_ID, items, title);
  }

  const report = buildReport(CO_ID, title, items, dodChecks);

  // Print table
  console.log(`\n${CO_ID} — ${title}`);
  console.log(`DoD: ${report.dod_pct}% (${report.passed}/${report.total} passed)\n`);
  for (const item of report.items) {
    const icon = item.status === 'pass' ? '✅' : item.status === 'pending' ? '⚠️' : '❌';
    console.log(`${icon} ${item.text}`);
    console.log(`   → ${item.evidence ?? 'no matching test or proof'}`);
  }

  // Save JSON output
  mkdirSync(DOD_OUTPUT_DIR, { recursive: true });
  const jsonPath = outputArg ?? join(DOD_OUTPUT_DIR, `${CO_ID}.json`);
  writeFileSync(jsonPath, JSON.stringify(report, null, 2));
  console.log(`\nDoD report saved: ${jsonPath}`);

  if (FLAG_POST_COMMENT) {
    await postPrComment(report);
  }

  // Exit non-zero only on real matched-test failures. Pending items
  // (stubs / no match / unmet proofs) are advisory — existence-only matching
  // cannot honestly block a merge.
  if (report.blocking_failures > 0) {
    process.exit(1);
  }
}

// Only run the CLI when invoked directly (not when imported by tests).
const invokedDirectly = (() => {
  const entry = argv[1];
  if (!entry) return false;
  try {
    return realpathSync(entry) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
})();

if (invokedDirectly) {
  main().catch(err => {
    console.error(err);
    process.exit(1);
  });
}
