/**
 * CO-372: Sprint PR cutoff check.
 *
 * Verifies that no PRs were merged after the sprint cutoff (Wednesday 23:59 BRT)
 * in the current sprint window. Called by scripts/release-commit.sh before the
 * release commit is created.
 *
 * Usage:
 *   node --import tsx scripts/scrum/cutoff-check.ts [--ignore-cutoff]
 *
 * Exit codes:
 *   0 — OK to release
 *   1 — Merges found after cutoff (unless --ignore-cutoff)
 */

import { execSync } from 'node:child_process';

const args = process.argv.slice(2);
const ignoreCutoff = args.includes('--ignore-cutoff');

// Anchor: sprint 0 ends 2026-06-11 at 15:00 BRT = 18:00 UTC
const ANCHOR_UTC_MS = Date.UTC(2026, 5, 11, 18, 0, 0); // month is 0-indexed
const SPRINT_DAYS_MS = 14 * 24 * 60 * 60 * 1000;

function sprintNumber(now: Date): number {
  const diffMs = now.getTime() - ANCHOR_UTC_MS;
  if (diffMs <= 0) return 0;
  return Math.floor((diffMs - 1) / SPRINT_DAYS_MS) + 1;
}

function sprintEndUtcMs(n: number): number {
  return ANCHOR_UTC_MS + n * SPRINT_DAYS_MS;
}

// Cutoff: Wednesday 23:59 BRT = Thursday 02:59 UTC
// Sprint ends Thursday 18:00 UTC. Cutoff = sprint_end - 15h01m
function cutoffUtcMs(sprintEndMs: number): number {
  return sprintEndMs - (15 * 60 + 1) * 60 * 1000;
}

const now = new Date();
const n = sprintNumber(now);
const endMs = sprintEndUtcMs(n);
const cutoffMs = cutoffUtcMs(endMs);
const cutoff = new Date(cutoffMs);
const cutoffIso = cutoff.toISOString();

let mergedAfterCutoff: string;
try {
  mergedAfterCutoff = execSync(
    `git log --merges --after="${cutoffIso}" --format="%H %s"`,
    { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] },
  ).trim();
} catch {
  // git log failure — treat as no merges found
  mergedAfterCutoff = '';
}

if (mergedAfterCutoff) {
  const nextSprintN = n + 1;
  if (ignoreCutoff) {
    process.stderr.write(
      `⚠️  WARNING: --ignore-cutoff override — merges after cutoff (${cutoffIso}):\n${mergedAfterCutoff}\n`,
    );
  } else {
    process.stderr.write(
      `❌ Merged after cutoff; rolls to Sprint ${nextSprintN}.\n` +
        `   Cutoff: ${cutoffIso} (Wednesday 23:59 BRT)\n` +
        `   Commits:\n${mergedAfterCutoff.split('\n').map(l => `     ${l}`).join('\n')}\n` +
        `   Use --ignore-cutoff for emergency hotfixes.\n`,
    );
    process.exit(1);
  }
} else {
  process.stderr.write(
    `✅ No merges after cutoff (${cutoffIso}) — Sprint ${n} release OK\n`,
  );
}
