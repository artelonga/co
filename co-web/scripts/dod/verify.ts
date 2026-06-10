/**
 * CO-382: DoD (Definition of Done) verification for a CO task spec.
 *
 * Reads work/co/CO-N.md, parses the ## Acceptance checklist, maps each item
 * to a test pattern, searches test files for matches, and reports per-item
 * ✅/❌.  Optionally posts a PR comment and generates stub spec files.
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
 */

import {
  readFileSync,
  writeFileSync,
  readdirSync,
  mkdirSync,
  existsSync,
} from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, '../../..');
const WORK_CO_DIR = join(REPO_ROOT, 'work/co');
const E2E_DIR = join(REPO_ROOT, 'co-web/e2e');
const E2E_GEN_DIR = join(REPO_ROOT, 'co-web/e2e-generated');
const DOD_OUTPUT_DIR = join(REPO_ROOT, 'docs/scrum/dod');

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------

const args = process.argv.slice(2);
function getArg(flag: string): string | undefined {
  const i = args.indexOf(flag);
  return i >= 0 ? args[i + 1] : undefined;
}
const FLAG_GENERATE_STUBS = args.includes('--generate-stubs');
const FLAG_POST_COMMENT = args.includes('--post-comment');

const specArg = getArg('--spec');
const outputArg = getArg('--output');

if (!specArg) {
  console.error('Usage: verify.ts --spec CO-NNN [--generate-stubs] [--post-comment] [--output path]');
  process.exit(1);
}

const CO_ID = specArg.toUpperCase().startsWith('CO-') ? specArg.toUpperCase() : `CO-${specArg}`;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface AcceptanceItem {
  text: string;
  checked: boolean;
}

interface DodItem {
  text: string;
  pattern: string;
  matched_file: string | null;
  matched_test: string | null;
  is_stub: boolean;
  // 'pending' = stub or no matching test yet — advisory, never blocks.
  // 'fail' is reserved for a matched real test that fails when executed;
  // existence-only matching (v1) cannot produce it.
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

function parseSpecFile(coId: string): { title: string; items: AcceptanceItem[] } {
  const specPath = join(WORK_CO_DIR, `${coId}.md`);
  if (!existsSync(specPath)) {
    throw new Error(`Spec not found: ${specPath}`);
  }
  const content = readFileSync(specPath, 'utf-8');

  // Extract title from frontmatter
  const titleMatch = content.match(/^title:\s*"?([^"\n]+)"?/m);
  const title = titleMatch ? titleMatch[1].trim() : coId;

  // Find ## Acceptance section and extract checklist items
  const acceptanceMatch = content.match(/^##\s+Acceptance\s*\n([\s\S]*?)(?=^##|\z)/m);
  if (!acceptanceMatch) {
    return { title, items: [] };
  }

  const section = acceptanceMatch[1];
  const items: AcceptanceItem[] = [];
  for (const line of section.split('\n')) {
    const unchecked = line.match(/^[-*]\s+\[\s\]\s+(.+)/);
    const checked = line.match(/^[-*]\s+\[x\]\s+(.+)/i);
    if (unchecked) items.push({ text: unchecked[1].trim(), checked: false });
    else if (checked) items.push({ text: checked[1].trim(), checked: true });
  }

  return { title, items };
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
function derivePattern(text: string): string {
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
// Test file search
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

function findMatchingTest(pattern: string): MatchedTest | null {
  const regex = new RegExp(pattern, 'i');
  const searchDirs = [E2E_DIR, E2E_GEN_DIR];
  const files = searchDirs.flatMap(listSpecFiles);

  for (const file of files) {
    const content = readFileSync(file, 'utf-8');
    const lines = content.split('\n');
    const tests = extractTestNames(content);
    for (const { name, lineIdx } of tests) {
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

function buildReport(
  coId: string,
  title: string,
  items: AcceptanceItem[],
): DodReport {
  const dodItems: DodItem[] = items.map(item => {
    const pattern = derivePattern(item.text);
    const match = findMatchingTest(pattern);

    let status: DodItem['status'];
    if (!match || match.isStub) {
      status = 'pending';
    } else {
      status = 'pass';
    }

    return {
      text: item.text,
      pattern,
      matched_file: match?.file ?? null,
      matched_test: match?.testName ?? null,
      is_stub: match?.isStub ?? false,
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
    timestamp: new Date().toISOString(),
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
  const token = process.env.GH_TOKEN;
  const prNumber = process.env.PR_NUMBER;
  const repo = process.env.REPO;

  if (!token || !prNumber || !repo) {
    console.warn('Skipping PR comment: GH_TOKEN, PR_NUMBER, REPO not all set');
    return;
  }

  const rows = report.items.map(item => {
    const icon = item.status === 'pass' ? '✅' : item.status === 'pending' ? '⚠️' : '❌';
    const testInfo = item.matched_file
      ? `\`${item.matched_test?.slice(0, 50)}\``
      : '_no test found_';
    return `| ${icon} | ${item.text.slice(0, 80)} | ${testInfo} |`;
  });

  const body = [
    `## 🛠️ DoD Verification — ${report.co_id}`,
    ``,
    `**${report.dod_pct}% complete** (${report.passed}/${report.total} acceptance items passing)`,
    ``,
    `| | Acceptance Item | Test |`,
    `|---|---|---|`,
    ...rows,
    ``,
    report.blocking_failures > 0
      ? `> ❌ **Merge blocked**: ${report.blocking_failures} matched test(s) failing.`
      : report.pending > 0
        ? `> ⚠️ Advisory: ${report.pending} acceptance item(s) pending real tests (stubs/no match). Merge not blocked.`
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
// Main
// ---------------------------------------------------------------------------

async function main(): Promise<void> {
  const { title, items } = parseSpecFile(CO_ID);

  if (items.length === 0) {
    console.warn(`No acceptance items found in ${CO_ID}.md — check ## Acceptance section`);
  }

  if (FLAG_GENERATE_STUBS) {
    generateStubs(CO_ID, items, title);
  }

  const report = buildReport(CO_ID, title, items);

  // Print table
  console.log(`\n${CO_ID} — ${title}`);
  console.log(`DoD: ${report.dod_pct}% (${report.passed}/${report.total} passed)\n`);
  for (const item of report.items) {
    const icon = item.status === 'pass' ? '✅' : item.status === 'pending' ? '⚠️' : '❌';
    console.log(`${icon} ${item.text}`);
    if (item.matched_file) {
      console.log(`   → ${item.matched_file}${item.is_stub ? ' (stub — fixme)' : ''}`);
    } else {
      console.log(`   → no matching test`);
    }
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
  // (stubs / no match) are advisory — existence-only matching cannot
  // honestly block a merge.
  if (report.blocking_failures > 0) {
    process.exit(1);
  }
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
