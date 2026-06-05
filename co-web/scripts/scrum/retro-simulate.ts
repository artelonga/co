/**
 * Retrospective sprint simulation for CO.
 *
 * Reconstructs CO's delivery history as bi-weekly sprints by walking
 * git log and work/co/CO-*.md spec files.
 *
 * Usage (from co-web/):
 *   npm run scrum:retro              — reconstruct past 12 + current sprint
 *   npm run scrum:retro -- --forward — also generate next 4 sprint templates
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
import { execSync } from 'node:child_process';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, '../../..');
const WORK_CO_DIR = join(REPO_ROOT, 'work/co');
const DOCS_SCRUM_DIR = join(REPO_ROOT, 'docs/scrum');
const SPRINTS_DIR = join(DOCS_SCRUM_DIR, 'sprints');

const FORWARD_MODE = process.argv.includes('--forward');
const FORWARD_COUNT = 4;

// Sprint 0 ends on 2026-06-11 (first going-forward anchor Thursday).
// Each sprint is a 14-day inclusive window: start = end - 13 days.
const ANCHOR_DATE = '2026-06-11';

// Commit subjects that are not feature deliveries — skip entirely.
const SKIP_SUBJECT = /^(chore\(archive\):|docs\(work\):|docs\(roadmap\):|chore\(release\):|chore\(tag\):)/;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface Spec {
  id: string;
  title: string;
  type: string;
  status: string;
  acceptanceChecks: { text: string; checked: boolean }[];
}

interface CommitEntry {
  date: string;       // YYYY-MM-DD
  isoDate: string;    // full ISO-8601
  subject: string;
  coId: string;       // "CO-123"
  prNumber: number | null;
}

interface TagEntry {
  name: string;  // "v2.39.0"
  date: string;  // YYYY-MM-DD
}

interface Pbi {
  id: string;
  title: string;
  status: 'done' | 'partial' | 'carried-over';
  merged_pr: number | null;
  merged_at: string | null;
  spec_file: string;
  acceptance_checks: { text: string; checked: boolean }[];
  release_tag: string | null;
}

interface Sprint {
  number: number;
  start: string;
  end: string;
  release_tags: string[];
  pbis: Pbi[];
}

interface SprintWindow {
  number: number;
  start: string;
  end: string;
}

// ---------------------------------------------------------------------------
// Date helpers
// ---------------------------------------------------------------------------

function addDays(dateStr: string, days: number): string {
  const d = new Date(dateStr + 'T12:00:00Z');
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

function isoToDate(isoStr: string): string {
  return isoStr.slice(0, 10);
}

function dateCmp(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

function isInWindow(date: string, start: string, end: string): boolean {
  return dateCmp(date, start) >= 0 && dateCmp(date, end) <= 0;
}

// ---------------------------------------------------------------------------
// Sprint window computation
// ---------------------------------------------------------------------------

function computeSprintWindows(): SprintWindow[] {
  const windows: SprintWindow[] = [];
  // Sprints -12 to 0 (12 completed past sprints + current in-flight)
  for (let n = -12; n <= 0; n++) {
    const end = addDays(ANCHOR_DATE, n * 14);
    const start = addDays(end, -13);
    windows.push({ number: n, start, end });
  }
  if (FORWARD_MODE) {
    for (let n = 1; n <= FORWARD_COUNT; n++) {
      const end = addDays(ANCHOR_DATE, n * 14);
      const start = addDays(end, -13);
      windows.push({ number: n, start, end });
    }
  }
  return windows;
}

// ---------------------------------------------------------------------------
// Spec parsing
// ---------------------------------------------------------------------------

function parseFrontmatter(content: string): Record<string, string> {
  const match = content.match(/^---\n([\s\S]*?)\n---/);
  if (!match) return {};
  const fm: Record<string, string> = {};
  for (const line of match[1].split('\n')) {
    const m = line.match(/^(\w+):\s*"?([^"#\n]*)"?\s*$/);
    if (m) fm[m[1].trim()] = m[2].trim();
  }
  return fm;
}

function parseAcceptanceChecks(
  content: string,
): { text: string; checked: boolean }[] {
  const checks: { text: string; checked: boolean }[] = [];
  const match = content.match(/## Acceptance\n([\s\S]*?)(?=\n## |\s*$)/);
  if (!match) return checks;
  for (const line of match[1].split('\n')) {
    const x = /^-\s+\[x\]\s+(.+)/i.exec(line);
    const o = /^-\s+\[\s*\]\s+(.+)/.exec(line);
    if (x) checks.push({ text: x[1].trim(), checked: true });
    else if (o) checks.push({ text: o[1].trim(), checked: false });
  }
  return checks;
}

function loadSpecs(): Map<string, Spec> {
  const specs = new Map<string, Spec>();
  let files: string[];
  try {
    files = readdirSync(WORK_CO_DIR);
  } catch {
    console.warn('[retro-simulate] Warning: work/co not found — no specs loaded');
    return specs;
  }
  for (const file of files) {
    if (!/^CO-\d+\.md$/.test(file)) continue;
    const content = readFileSync(join(WORK_CO_DIR, file), 'utf8');
    const fm = parseFrontmatter(content);
    const num = file.replace(/^CO-(\d+)\.md$/, '$1');
    const id = `CO-${fm['id'] || num}`;
    specs.set(id, {
      id,
      title: fm['title'] || '(untitled)',
      type: fm['type'] || 'unknown',
      status: fm['status'] || 'todo',
      acceptanceChecks: parseAcceptanceChecks(content),
    });
  }
  return specs;
}

// ---------------------------------------------------------------------------
// Git operations
// ---------------------------------------------------------------------------

function git(cmd: string): string {
  return execSync(cmd, {
    cwd: REPO_ROOT,
    encoding: 'utf8',
    stdio: ['pipe', 'pipe', 'pipe'],
  });
}

function loadCommits(): CommitEntry[] {
  // %aI = author date ISO-8601, %x09 = tab, %s = subject
  const raw = git(
    'git log --first-parent main --pretty=format:"%aI%x09%s"',
  );
  const entries: CommitEntry[] = [];
  const seen = new Set<string>();

  for (const line of raw.split('\n')) {
    if (!line.trim()) continue;
    const tab = line.indexOf('\t');
    if (tab === -1) continue;
    const isoDate = line.slice(0, tab).replace(/^"|"$/g, '').trim();
    const subject = line.slice(tab + 1).trim();

    if (SKIP_SUBJECT.test(subject)) continue;

    // Extract first CO-N reference (single ID only — skip "CO-368/369/..." multi-refs)
    if (/\bCO-\d+\/\d+/.test(subject)) continue;
    const coMatch = subject.match(/\bCO-(\d+)\b/);
    if (!coMatch) continue;

    const coId = `CO-${coMatch[1]}`;
    if (seen.has(coId)) continue; // first (most recent) commit per CO-N wins
    seen.add(coId);

    const prMatch = subject.match(/#(\d+)\b/);
    const prNumber = prMatch ? parseInt(prMatch[1], 10) : null;

    entries.push({
      date: isoToDate(isoDate),
      isoDate,
      subject,
      coId,
      prNumber,
    });
  }

  return entries;
}

function loadTags(): TagEntry[] {
  // Note: for-each-ref does not support %x09 — use | as separator (safe for v* tag names).
  const raw = git(
    'git for-each-ref --format="%(refname:short)|%(creatordate:iso8601)" refs/tags',
  );
  const entries: TagEntry[] = [];
  for (const line of raw.split('\n')) {
    if (!line.trim()) continue;
    const pipe = line.indexOf('|');
    if (pipe === -1) continue;
    const name = line.slice(0, pipe).replace(/^"|"$/g, '').trim();
    const dateStr = line.slice(pipe + 1).trim();
    if (!/^v\d+\.\d+/.test(name)) continue;
    entries.push({ name, date: isoToDate(dateStr) });
  }
  return entries.sort((a, b) => dateCmp(a.date, b.date));
}

// ---------------------------------------------------------------------------
// Sprint assembly
// ---------------------------------------------------------------------------

function derivePbiStatus(spec: Spec | undefined): Pbi['status'] {
  if (!spec) return 'done';
  if (spec.status === 'done') return 'done';
  const checks = spec.acceptanceChecks;
  if (checks.length > 0 && checks.some(c => c.checked)) return 'partial';
  return 'done'; // commit is the proof of delivery
}

function buildSprints(
  windows: SprintWindow[],
  commits: CommitEntry[],
  tags: TagEntry[],
  specs: Map<string, Spec>,
): Sprint[] {
  return windows.map(w => {
    const windowCommits = commits.filter(c => isInWindow(c.date, w.start, w.end));
    const windowTags = tags
      .filter(t => isInWindow(t.date, w.start, w.end))
      .sort((a, b) => dateCmp(a.date, b.date));

    const pbis: Pbi[] = windowCommits.map(c => {
      const spec = specs.get(c.coId);
      const checks = spec?.acceptanceChecks ?? [];
      // Assign the chronologically next tag in this window after the commit
      const nextTag = windowTags.find(t => dateCmp(t.date, c.date) >= 0);
      return {
        id: c.coId,
        title: spec?.title ?? '(spec not found)',
        status: derivePbiStatus(spec),
        merged_pr: c.prNumber,
        merged_at: c.date,
        spec_file: spec ? `work/co/${c.coId}.md` : '',
        acceptance_checks: checks,
        release_tag: nextTag?.name ?? (windowTags.length > 0 ? windowTags[windowTags.length - 1].name : null),
      };
    });

    return {
      number: w.number,
      start: w.start,
      end: w.end,
      release_tags: windowTags.map(t => t.name),
      pbis,
    };
  });
}

// ---------------------------------------------------------------------------
// Output formatters
// ---------------------------------------------------------------------------

function sprintFileName(n: number): string {
  return n < 0 ? `sprint-minus-${Math.abs(n)}.md` : `sprint-${n}.md`;
}

function sprintLabel(n: number): string {
  if (n === 0) return 'Sprint 0';
  return `Sprint ${n}`;
}

function formatSprintMd(sprint: Sprint): string {
  const label = sprintLabel(sprint.number);
  const range = `${sprint.start} → ${sprint.end}`;
  const releaseStr =
    sprint.release_tags.length > 0
      ? sprint.release_tags[sprint.release_tags.length - 1]
      : '(no release in this window)';
  const velocity = sprint.pbis.length;
  const lines: string[] = [];

  lines.push(`# ${label} (${range})`);
  lines.push('');
  lines.push('**Sprint Goal**: (retrospective — inferred from PBIs)');
  lines.push(`**Release**: ${releaseStr}`);
  lines.push(`**Velocity**: ${velocity} PBI${velocity !== 1 ? 's' : ''} delivered`);
  lines.push('');

  if (sprint.number > 0) {
    lines.push('## Planned PBIs');
    lines.push('');
    lines.push('_(not yet planned)_');
    lines.push('');
    lines.push('## Sprint Goal Notes');
    lines.push('');
    lines.push('_(to be defined at sprint planning)_');
    lines.push('');
  } else {
    lines.push('## Delivered PBIs');
    lines.push('');

    if (sprint.pbis.length === 0) {
      lines.push('_(no PBIs tracked in this window)_');
      lines.push('');
    } else {
      for (const pbi of sprint.pbis) {
        const prStr = pbi.merged_pr ? ` (#${pbi.merged_pr})` : '';
        lines.push(`### ${pbi.id} — ${pbi.title}${prStr}`);
        if (pbi.merged_at) lines.push(`_Merged: ${pbi.merged_at}_`);
        if (pbi.release_tag) lines.push(`_Release: ${pbi.release_tag}_`);
        lines.push('');
        if (pbi.acceptance_checks.length > 0) {
          for (const ch of pbi.acceptance_checks) {
            lines.push(`- [${ch.checked ? 'x' : ' '}] ${ch.text}`);
          }
        } else {
          lines.push('_(no acceptance criteria in spec)_');
        }
        lines.push('');
      }
    }

    lines.push('## Carried Over');
    lines.push('');
    lines.push('- (none tracked — retrospective simulation uses merge commits only)');
    lines.push('');
  }

  return lines.join('\n');
}

function formatVelocityMd(sprints: Sprint[]): string {
  const retro = sprints.filter(s => s.number <= 0);
  const lines: string[] = [];
  lines.push('# Velocity Chart — Retrospective Simulation');
  lines.push('');
  lines.push('PBIs delivered per sprint, reconstructed from git history.');
  lines.push('');
  lines.push('| Sprint | Period | PBIs | Bar |');
  lines.push('|--------|--------|-----:|-----|');

  const max = Math.max(...retro.map(s => s.pbis.length), 1);
  for (const s of retro) {
    const n = s.pbis.length;
    const bar = '█'.repeat(Math.max(1, Math.round((n / max) * 20)));
    const label = s.number === 0 ? 'Sprint 0 (in-flight)' : `Sprint ${s.number}`;
    lines.push(`| ${label} | ${s.start} → ${s.end} | ${n} | ${n > 0 ? bar : '—'} |`);
  }

  const completed = retro.filter(s => s.number < 0);
  const totalPbis = completed.reduce((sum, s) => sum + s.pbis.length, 0);
  const avg =
    completed.length > 0
      ? (totalPbis / completed.length).toFixed(1)
      : 'N/A';

  lines.push('');
  lines.push(`**Total PBIs reconstructed (past sprints)**: ${totalPbis}`);
  lines.push(`**Average velocity**: ${avg} PBIs/sprint`);
  lines.push(
    `**Sprint 0 (in-flight)**: ${sprints.find(s => s.number === 0)?.pbis.length ?? 0} PBIs so far`,
  );
  lines.push('');
  lines.push(
    '> Generated by `scripts/scrum/retro-simulate.ts`. Run `npm run scrum:retro` to refresh.',
  );
  lines.push('');

  return lines.join('\n');
}

function formatAnomaliesMd(
  sprints: Sprint[],
  commits: CommitEntry[],
  specs: Map<string, Spec>,
): string {
  const lines: string[] = [];
  lines.push('# Retrospective Anomalies');
  lines.push('');
  lines.push(
    'Patterns flagged during reconstruction. Data quality signals, not bugs.',
  );
  lines.push('');

  // Commits without matching spec files
  const noSpec = commits.filter(c => !specs.has(c.coId));
  lines.push('## Commits with no matching spec file');
  lines.push('');
  if (noSpec.length === 0) {
    lines.push(
      '_(none — all delivery commits reference a spec in `work/co/`)_',
    );
  } else {
    for (const c of noSpec) {
      lines.push(
        `- **${c.coId}** (${c.date}) — \`${c.subject.slice(0, 80)}\``,
      );
    }
  }
  lines.push('');

  // Sprints with no release tag
  const retroSprints = sprints.filter(s => s.number < 0);
  const noRelease = retroSprints.filter(s => s.release_tags.length === 0);
  lines.push('## Past sprints with no release tag');
  lines.push('');
  if (noRelease.length === 0) {
    lines.push('_(none — all past sprints contain at least one release tag)_');
  } else {
    for (const s of noRelease) {
      lines.push(
        `- Sprint ${s.number} (${s.start} → ${s.end}): ${s.pbis.length} PBIs delivered, no tag`,
      );
    }
  }
  lines.push('');

  // Specs marked done with no delivery commit found
  const allDelivered = new Set(sprints.flatMap(s => s.pbis.map(p => p.id)));
  const doneUntracked: Spec[] = [];
  for (const spec of specs.values()) {
    if (spec.status === 'done' && !allDelivered.has(spec.id)) {
      doneUntracked.push(spec);
    }
  }
  lines.push('## Specs marked `done` with no delivery commit found');
  lines.push('');
  if (doneUntracked.length === 0) {
    lines.push('_(none)_');
  } else {
    const shown = doneUntracked.slice(0, 25);
    for (const spec of shown) {
      lines.push(`- **${spec.id}** — ${spec.title}`);
    }
    if (doneUntracked.length > 25) {
      lines.push(`- _(${doneUntracked.length - 25} more not shown)_`);
    }
  }
  lines.push('');

  // Sprints with unusually high release frequency (>5 tags)
  const dense = retroSprints.filter(s => s.release_tags.length > 5);
  if (dense.length > 0) {
    lines.push('## Sprints with high release frequency (>5 tags)');
    lines.push('');
    for (const s of dense) {
      lines.push(
        `- Sprint ${s.number}: ${s.release_tags.length} releases — ${s.release_tags.join(', ')}`,
      );
    }
    lines.push('');
  }

  lines.push(
    '> Generated by `scripts/scrum/retro-simulate.ts`. Run `npm run scrum:retro` to refresh.',
  );
  lines.push('');

  return lines.join('\n');
}

function formatSimulationMd(sprints: Sprint[]): string {
  const retro = sprints.filter(s => s.number <= 0);
  const lines: string[] = [];

  lines.push('# Retrospective Sprint Simulation — CO');
  lines.push('');
  lines.push(
    "CO has been shipping continuously since v0.1. This document reconstructs that history",
  );
  lines.push(
    'as bi-weekly sprints, providing the data foundation for velocity measurement,',
  );
  lines.push('retrospectives, and sprint calendar rendering (CO-372).');
  lines.push('');
  lines.push('## Cadence Math');
  lines.push('');
  lines.push('| Param | Value |');
  lines.push('|-------|-------|');
  lines.push('| Anchor | 2026-06-11 (first going-forward Sprint 1 review, Thursday 15:00 BRT) |');
  lines.push('| Sprint duration | 14 days (inclusive) |');
  lines.push('| Sprint 0 | 2026-05-29 → 2026-06-11 (current/in-flight) |');
  lines.push('| Sprint -1 | 2026-05-15 → 2026-05-28 (last completed) |');
  lines.push('| Earliest reconstructed | Sprint -12: 2025-12-12 → 2025-12-25 |');
  lines.push('');
  lines.push('## Sprint Summary');
  lines.push('');
  lines.push('| Sprint | Period | PBIs | Latest release |');
  lines.push('|--------|--------|-----:|----------------|');
  for (const s of retro) {
    const label = s.number === 0 ? 'Sprint 0 (in-flight)' : `Sprint ${s.number}`;
    const latest =
      s.release_tags.length > 0
        ? s.release_tags[s.release_tags.length - 1]
        : '—';
    lines.push(`| ${label} | ${s.start} → ${s.end} | ${s.pbis.length} | ${latest} |`);
  }
  lines.push('');
  lines.push('## Sprint Files');
  lines.push('');
  for (const s of retro) {
    const file = sprintFileName(s.number);
    const label =
      s.number === 0 ? 'Sprint 0 — current (in-flight)' : `Sprint ${s.number}`;
    lines.push(`- [${label}](sprints/${file})`);
  }
  lines.push('');
  lines.push('## Methodology Notes');
  lines.push('');
  lines.push(
    '- **PBI extraction**: `git log --first-parent main` commits with a `CO-N` reference.',
  );
  lines.push(
    '  Archive commits (`chore(archive):`), docs commits (`docs(work):`), and release commits',
  );
  lines.push('  (`chore(release):`) are excluded.');
  lines.push(
    '- **Sprint Increment**: the latest `v*` tag within each sprint window.',
  );
  lines.push(
    '- **DoD**: parsed from `## Acceptance` checklists in `work/co/CO-N.md` spec files.',
  );
  lines.push(
    '  Checkbox state reflects current spec files (most specs leave items as `[ ]` after delivery).',
  );
  lines.push(
    '- **Sprint goal**: `null` for all retrospective sprints — inferred from PBI titles.',
  );
  lines.push(
    '- This file is regenerated by `npm run scrum:retro` from `co-web/`. Idempotent.',
  );
  lines.push('');

  return lines.join('\n');
}

function buildIndex(sprints: Sprint[]): object {
  return sprints.map(s => ({
    number: s.number,
    start: s.start,
    end: s.end,
    release_tags: s.release_tags,
    pbi_count: s.pbis.length,
    pbis: s.pbis.map(p => ({
      id: p.id,
      title: p.title,
      status: p.status,
      merged_pr: p.merged_pr,
      merged_at: p.merged_at,
      release_tag: p.release_tag,
    })),
  }));
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

function ensureDir(dir: string) {
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
}

function writeIfChanged(path: string, content: string) {
  if (existsSync(path) && readFileSync(path, 'utf8') === content) return;
  writeFileSync(path, content, 'utf8');
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function main() {
  console.log('[retro-simulate] Loading specs from work/co/...');
  const specs = loadSpecs();
  console.log(`[retro-simulate]   ${specs.size} spec files`);

  console.log('[retro-simulate] Reading git log (main)...');
  const commits = loadCommits();
  console.log(`[retro-simulate]   ${commits.length} CO delivery commits`);

  console.log('[retro-simulate] Reading git tags...');
  const tags = loadTags();
  console.log(`[retro-simulate]   ${tags.length} version tags`);

  console.log('[retro-simulate] Computing sprint windows...');
  const windows = computeSprintWindows();
  const sprints = buildSprints(windows, commits, tags, specs);
  const retroCount = sprints.filter(s => s.number <= 0).length;
  const fwdCount = sprints.filter(s => s.number > 0).length;
  console.log(
    `[retro-simulate]   ${retroCount} retrospective sprints, ${fwdCount} forward templates`,
  );

  ensureDir(SPRINTS_DIR);

  for (const sprint of sprints) {
    writeIfChanged(
      join(SPRINTS_DIR, sprintFileName(sprint.number)),
      formatSprintMd(sprint),
    );
  }

  writeIfChanged(
    join(DOCS_SCRUM_DIR, 'retro-simulation.md'),
    formatSimulationMd(sprints),
  );
  writeIfChanged(
    join(DOCS_SCRUM_DIR, 'retro-velocity.md'),
    formatVelocityMd(sprints),
  );
  writeIfChanged(
    join(DOCS_SCRUM_DIR, 'retro-anomalies.md'),
    formatAnomaliesMd(sprints, commits, specs),
  );
  writeIfChanged(
    join(SPRINTS_DIR, '_index.json'),
    JSON.stringify(buildIndex(sprints), null, 2) + '\n',
  );

  const totalPbis = sprints.reduce((s, sp) => s + sp.pbis.length, 0);
  console.log(`[retro-simulate] ✓ ${sprints.length} sprint files written`);
  console.log(`[retro-simulate] ✓ ${totalPbis} PBIs reconstructed total`);
  console.log('[retro-simulate] ✓ retro-simulation.md, retro-velocity.md, retro-anomalies.md');
  console.log('[retro-simulate] ✓ sprints/_index.json');
  if (FORWARD_MODE) {
    console.log(`[retro-simulate] ✓ forward mode: sprints 1–${FORWARD_COUNT} generated`);
  }
}

main();
