/**
 * CO-382: Sprint review generator.
 *
 * Reads DoD JSON outputs from docs/scrum/dod/, combines with git/spec data,
 * and generates a sprint review markdown at docs/scrum/sprints/sprint-<N>.md.
 *
 * Runs Thursday 14:30 BRT (17:30 UTC) via release-gate.yml cron.
 * After generating, commits the file so the sprint record is versioned.
 *
 * Usage (from co-web/):
 *   node --import tsx scripts/scrum/sprint-review.ts
 *   node --import tsx scripts/scrum/sprint-review.ts --sprint 1
 *   node --import tsx scripts/scrum/sprint-review.ts --dry-run
 */

import {
  readFileSync,
  writeFileSync,
  readdirSync,
  existsSync,
  mkdirSync,
} from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execSync } from 'node:child_process';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, '../../..');
const DOD_DIR = join(REPO_ROOT, 'docs/scrum/dod');
const SPRINTS_DIR = join(REPO_ROOT, 'docs/scrum/sprints');
const WORK_CO_DIR = join(REPO_ROOT, 'work/co');

const args = process.argv.slice(2);
function getArg(flag: string): string | undefined {
  const i = args.indexOf(flag);
  return i >= 0 ? args[i + 1] : undefined;
}
const DRY_RUN = args.includes('--dry-run');
const SPRINT_ARG = getArg('--sprint');

// Sprint 0 anchor (2026-06-11, first going-forward Thursday)
const ANCHOR_DATE = '2026-06-11';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface DodItem {
  text: string;
  status: 'pass' | 'fail' | 'no-test';
}

interface DodReport {
  co_id: string;
  spec_title: string;
  timestamp: string;
  items: DodItem[];
  total: number;
  passed: number;
  failed: number;
  dod_pct: number;
}

interface PbiReview {
  co_id: string;
  title: string;
  dod_pct: number;
  passed: number;
  total: number;
  merged_pr: number | null;
  release_tag: string | null;
  status: 'delivered' | 'partial' | 'carried-over';
}

interface SprintReview {
  sprint_number: number;
  start: string;
  end: string;
  velocity: number;
  avg_dod_pct: number;
  pbis: PbiReview[];
  release_tags: string[];
  goal: string;
}

// ---------------------------------------------------------------------------
// Date helpers
// ---------------------------------------------------------------------------

function addDays(dateStr: string, days: number): string {
  const d = new Date(dateStr + 'T12:00:00Z');
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

function currentSprintNumber(): number {
  const today = new Date().toISOString().slice(0, 10);
  const anchor = new Date(ANCHOR_DATE + 'T12:00:00Z');
  const now = new Date(today + 'T12:00:00Z');
  const diffDays = Math.floor((now.getTime() - anchor.getTime()) / (1000 * 86400));
  return Math.floor(diffDays / 14);
}

function sprintWindow(n: number): { start: string; end: string } {
  const end = addDays(ANCHOR_DATE, n * 14);
  const start = addDays(end, -13);
  return { start, end };
}

// ---------------------------------------------------------------------------
// DoD report loading
// ---------------------------------------------------------------------------

function loadDodReports(): Map<string, DodReport> {
  const map = new Map<string, DodReport>();
  if (!existsSync(DOD_DIR)) return map;

  for (const f of readdirSync(DOD_DIR)) {
    if (!f.endsWith('.json')) continue;
    try {
      const raw = readFileSync(join(DOD_DIR, f), 'utf-8');
      const report = JSON.parse(raw) as DodReport;
      map.set(report.co_id, report);
    } catch {
      // skip malformed
    }
  }
  return map;
}

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

function gitLog(since: string, until: string): Array<{ subject: string; date: string }> {
  try {
    const out = execSync(
      `git log --format="%cd|%s" --date=short --after="${since}" --until="${until}"`,
      { cwd: REPO_ROOT, encoding: 'utf-8' },
    );
    return out
      .trim()
      .split('\n')
      .filter(Boolean)
      .map(line => {
        const [date, ...rest] = line.split('|');
        return { date: date.trim(), subject: rest.join('|').trim() };
      });
  } catch {
    return [];
  }
}

function latestTags(since: string, until: string): string[] {
  try {
    const out = execSync(
      `git tag --sort=-creatordate --list "v*"`,
      { cwd: REPO_ROOT, encoding: 'utf-8' },
    );
    return out
      .trim()
      .split('\n')
      .filter(Boolean)
      .filter(tag => {
        try {
          const d = execSync(`git log -1 --format="%ad" --date=short ${tag}`, {
            cwd: REPO_ROOT,
            encoding: 'utf-8',
          }).trim();
          return d >= since && d <= until;
        } catch {
          return false;
        }
      });
  } catch {
    return [];
  }
}

// ---------------------------------------------------------------------------
// Spec lookup
// ---------------------------------------------------------------------------

function specTitle(coId: string): string {
  const path = join(WORK_CO_DIR, `${coId}.md`);
  if (!existsSync(path)) return coId;
  const m = readFileSync(path, 'utf-8').match(/^title:\s*"?([^"\n]+)"?/m);
  return m ? m[1].trim() : coId;
}

function extractPrNumber(subject: string): number | null {
  const m = subject.match(/#(\d+)\)/);
  return m ? parseInt(m[1], 10) : null;
}

function extractCoId(subject: string): string | null {
  const m = subject.match(/\bCO-(\d+)\b/);
  return m ? `CO-${m[1]}` : null;
}

// ---------------------------------------------------------------------------
// Sprint review generation
// ---------------------------------------------------------------------------

function buildSprintReview(n: number): SprintReview {
  const { start, end } = sprintWindow(n);
  const commits = gitLog(start, end);
  const dodReports = loadDodReports();
  const tags = latestTags(start, end);

  const pbis: PbiReview[] = [];
  const seen = new Set<string>();

  for (const commit of commits) {
    const coId = extractCoId(commit.subject);
    if (!coId || seen.has(coId)) continue;
    seen.add(coId);

    const dod = dodReports.get(coId);
    const title = dod?.spec_title ?? specTitle(coId);
    const dodPct = dod?.dod_pct ?? 0;
    const prNumber = extractPrNumber(commit.subject);

    let status: PbiReview['status'];
    if (dodPct >= 100) status = 'delivered';
    else if (dodPct > 0) status = 'partial';
    else status = 'carried-over';

    pbis.push({
      co_id: coId,
      title,
      dod_pct: dodPct,
      passed: dod?.passed ?? 0,
      total: dod?.total ?? 0,
      merged_pr: prNumber,
      release_tag: tags.length > 0 ? tags[0] : null,
      status,
    });
  }

  const delivered = pbis.filter(p => p.status === 'delivered').length;
  const avgDod =
    pbis.length > 0
      ? Math.round(pbis.reduce((s, p) => s + p.dod_pct, 0) / pbis.length)
      : 100;

  return {
    sprint_number: n,
    start,
    end,
    velocity: delivered,
    avg_dod_pct: avgDod,
    pbis,
    release_tags: tags,
    goal: `Sprint ${n} delivery`,
  };
}

// ---------------------------------------------------------------------------
// Markdown rendering
// ---------------------------------------------------------------------------

function renderSprintReview(sr: SprintReview): string {
  const statusIcon = (s: PbiReview['status']) =>
    s === 'delivered' ? '✅' : s === 'partial' ? '⚠️' : '🔁';

  const pbiRows = sr.pbis.map(p => {
    const prLink = p.merged_pr ? `[#${p.merged_pr}]` : '—';
    const dod = p.total > 0 ? `${p.dod_pct}% (${p.passed}/${p.total})` : '—';
    return `| ${statusIcon(p.status)} | ${p.co_id} | ${p.title.slice(0, 60)} | ${dod} | ${prLink} |`;
  });

  const lines = [
    `# Sprint ${sr.sprint_number} Review (${sr.start} → ${sr.end})`,
    ``,
    `**Sprint Goal**: ${sr.goal}`,
    sr.release_tags.length > 0
      ? `**Release**: ${sr.release_tags.join(', ')}`
      : `**Release**: (pending)`,
    `**Velocity**: ${sr.velocity} PBIs delivered`,
    `**Avg DoD**: ${sr.avg_dod_pct}%`,
    ``,
    `## Delivered PBIs`,
    ``,
    `| | ID | Title | DoD | PR |`,
    `|---|---|---|---|---|`,
    ...pbiRows,
    ``,
    `## Metrics`,
    ``,
    `| Metric | Value |`,
    `|---|---|`,
    `| PBIs delivered | ${sr.velocity} |`,
    `| PBIs partial | ${sr.pbis.filter(p => p.status === 'partial').length} |`,
    `| PBIs carried over | ${sr.pbis.filter(p => p.status === 'carried-over').length} |`,
    `| Average DoD % | ${sr.avg_dod_pct}% |`,
    ``,
    `_Generated by \`scripts/scrum/sprint-review.ts\` — ${new Date().toISOString()}_`,
  ];

  return lines.join('\n');
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function main(): void {
  const n = SPRINT_ARG != null ? parseInt(SPRINT_ARG, 10) : currentSprintNumber();
  console.log(`Generating Sprint ${n} review…`);

  const review = buildSprintReview(n);
  const md = renderSprintReview(review);

  mkdirSync(SPRINTS_DIR, { recursive: true });
  const outPath = join(SPRINTS_DIR, `sprint-${n}.md`);

  if (DRY_RUN) {
    console.log('\n--- DRY RUN OUTPUT ---\n');
    console.log(md);
    return;
  }

  writeFileSync(outPath, md);
  console.log(`Sprint review written: ${outPath}`);

  // Commit the sprint review
  try {
    execSync(`git add "${outPath}"`, { cwd: REPO_ROOT });
    execSync(
      `git commit -m "docs(scrum): Sprint ${n} review — ${new Date().toISOString().slice(0, 10)}\n\nCo-Authored-By: Claude <noreply@anthropic.com>"`,
      { cwd: REPO_ROOT },
    );
    console.log(`Committed sprint review for sprint ${n}`);
  } catch (e) {
    console.warn('Git commit skipped (nothing to commit or not in a git repo)');
  }
}

main();
