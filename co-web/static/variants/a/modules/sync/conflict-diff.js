// CO-128: line-level diff for the conflict-resolution modal.
//
// Pure, dependency-free LCS line diff. Produces aligned side-by-side rows so
// the modal can render a monospace two-column body (Apple-Finder style).
//
// Left column = local (your version), right column = remote (server version).

// Spec risk mitigation: cap each side so the O(n·m) LCS never chokes on a huge
// markdown file. The server already caps at 100 KB (CONFLICT_BODY_CAP); this is
// a second guard for safety when the modal is fed an in-memory payload.
export const DIFF_BODY_CAP = 100 * 1024;
const MAX_DIFF_LINES = 4000;

/**
 * @typedef {Object} DiffRow
 * @property {'equal'|'del'|'add'} kind
 *   `equal` — line unchanged; `del` — only in local; `add` — only in remote.
 * @property {string|null} left   Local line (null when `add`).
 * @property {string|null} right  Remote line (null when `del`).
 */

/**
 * Compute an aligned side-by-side line diff between `local` and `remote`.
 *
 * @param {string} local   The local (your) version.
 * @param {string} remote  The remote (server) version.
 * @returns {{ rows: DiffRow[], truncated: boolean, added: number, removed: number }}
 */
export function lineDiff(local, remote) {
    const a = String(local ?? '').slice(0, DIFF_BODY_CAP).split('\n');
    const b = String(remote ?? '').slice(0, DIFF_BODY_CAP).split('\n');

    // Too many lines for an interactive diff — return a flat summary instead.
    if (a.length + b.length > MAX_DIFF_LINES) {
        return {
            rows: [{ kind: 'equal', left: a.join('\n'), right: b.join('\n') }],
            truncated: true,
            added: 0,
            removed: 0,
        };
    }

    // LCS length table.
    const n = a.length;
    const m = b.length;
    const lcs = Array.from({ length: n + 1 }, () => new Int32Array(m + 1));
    for (let i = n - 1; i >= 0; i--) {
        for (let j = m - 1; j >= 0; j--) {
            lcs[i][j] = a[i] === b[j]
                ? lcs[i + 1][j + 1] + 1
                : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
        }
    }

    // Backtrack into aligned rows.
    const rows = [];
    let added = 0;
    let removed = 0;
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
        if (a[i] === b[j]) {
            rows.push({ kind: 'equal', left: a[i], right: b[j] });
            i++;
            j++;
        } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
            rows.push({ kind: 'del', left: a[i], right: null });
            removed++;
            i++;
        } else {
            rows.push({ kind: 'add', left: null, right: b[j] });
            added++;
            j++;
        }
    }
    while (i < n) {
        rows.push({ kind: 'del', left: a[i], right: null });
        removed++;
        i++;
    }
    while (j < m) {
        rows.push({ kind: 'add', left: null, right: b[j] });
        added++;
        j++;
    }

    return { rows, truncated: false, added, removed };
}
