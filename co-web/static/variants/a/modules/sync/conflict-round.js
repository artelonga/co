// CO-128: conflict-resolution "round" controller.
//
// A sync round can surface several conflicts at once. This controller walks the
// queue, opening the Apple-style modal for each conflict and wiring the chosen
// action to the real entry API. The "Apply to all" checkbox makes one choice
// sticky for the rest of the round — held in component state only, so it never
// survives a reload (per CO-128 acceptance).
//
// Action → effect (matches the CO-128 modal-behaviors table):
//   ignore     server keeps remote; local discarded (no write)
//   replace    server takes local; remote discarded (PUT with remote's hash as base)
//   keep_both  server creates `<name>.local.md`; both persist
//   cancel     abort the whole round (Esc)

import { openConflictModal } from './conflict-modal.js';

/**
 * Derive the "keep both" path: insert `.local` before the final extension.
 *   notes/x.md      → notes/x.local.md
 *   notes/x         → notes/x.local
 */
export function localVariantPath(path) {
    const p = String(path || '');
    const slash = p.lastIndexOf('/');
    const dot = p.lastIndexOf('.');
    if (dot > slash) {
        return `${p.slice(0, dot)}.local${p.slice(dot)}`;
    }
    return `${p}.local`;
}

function encodePath(path) {
    return String(path || '')
        .split('/')
        .map(encodeURIComponent)
        .join('/');
}

// Default API handlers. Each returns a promise resolving to a small result
// object. Injectable so the round controller is unit-testable without a server.
export const defaultConflictApi = {
    // Keep the remote version — nothing to write server-side.
    async ignore(conflict) {
        return { ok: true, action: 'ignore', path: conflict.path };
    },

    // Local wins: re-PUT the local body using the remote hash as the base so it
    // passes the optimistic-concurrency check and overwrites the server copy.
    async replace(conflict) {
        const slug = conflict.universe_key;
        const res = await window.fetch(
            `/api/v1/universes/${encodeURIComponent(slug)}/entries/${encodePath(conflict.path)}`,
            {
                method: 'PUT',
                credentials: 'same-origin',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    body: conflict.local.body,
                    base_hash: conflict.remote.body_hash,
                }),
            },
        );
        let body_hash = null;
        if (res.ok) {
            const json = await res.json().catch(() => null);
            body_hash = json && json.body_hash;
        }
        return { ok: res.ok, action: 'replace', status: res.status, path: conflict.path, body_hash };
    },

    // Both survive: create a sibling `<name>.local.md` holding the local body.
    async keepBoth(conflict) {
        const slug = conflict.universe_key;
        const newPath = localVariantPath(conflict.path);
        const base = newPath.split('/').pop().replace(/\.[^.]+$/, '');
        const res = await window.fetch(
            `/api/v1/universes/${encodeURIComponent(slug)}/entries`,
            {
                method: 'POST',
                credentials: 'same-origin',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    path: newPath,
                    frontmatter: { type: 'note', title: `${base} (local)` },
                    body: conflict.local.body,
                }),
            },
        );
        return {
            ok: res.ok,
            action: 'keep_both',
            status: res.status,
            path: conflict.path,
            local_path: newPath,
        };
    },
};

/**
 * Drives a single sync round of conflict resolution.
 */
export class ConflictRound {
    /**
     * @param {object} [opts]
     * @param {(payload: object, o: object) => Promise<{action: string, applyToAll: boolean}>} [opts.openModal]
     * @param {object} [opts.api]  Action handlers (see {@link defaultConflictApi}).
     * @param {(conflict: object, action: string, result: object) => void} [opts.onResolved]
     */
    constructor(opts = {}) {
        this.openModal = opts.openModal || openConflictModal;
        this.api = opts.api || defaultConflictApi;
        this.onResolved = opts.onResolved || null;
        this.queue = [];
        this.results = [];
        /** Sticky action set once "Apply to all" is checked — round-scoped only. */
        this.stickyAction = null;
        this.cancelled = false;
    }

    /** Add a conflict to this round's queue. */
    enqueue(conflict) {
        this.queue.push(conflict);
        return this;
    }

    /** Whether "Apply to all" is currently active for this round. */
    get applyToAllActive() {
        return this.stickyAction !== null;
    }

    /**
     * Process the queue. Resolves with `{ results, cancelled }`.
     */
    async run() {
        while (this.queue.length > 0) {
            const conflict = this.queue.shift();

            let action = this.stickyAction;
            // No sticky choice yet → ask the user.
            if (action === null) {
                const choice = await this.openModal(conflict, {
                    applyToAllChecked: false,
                    remaining: this.queue.length,
                });
                action = choice.action;
                if (choice.applyToAll && action !== 'cancel') {
                    this.stickyAction = action;
                }
            }

            if (action === 'cancel') {
                this.cancelled = true;
                break;
            }

            const result = await this.apply(conflict, action);
            this.results.push({ conflict, action, result });
            if (this.onResolved) this.onResolved(conflict, action, result);
        }
        return { results: this.results, cancelled: this.cancelled };
    }

    /** Dispatch a single resolved action to the API layer. */
    apply(conflict, action) {
        switch (action) {
            case 'ignore': return this.api.ignore(conflict);
            case 'replace': return this.api.replace(conflict);
            case 'keep_both': return this.api.keepBoth(conflict);
            default: return Promise.resolve({ ok: false, action, error: 'unknown_action' });
        }
    }
}

/**
 * Convenience: resolve a batch of conflicts in one round.
 *
 * @param {object[]} conflicts
 * @param {object} [opts]  Forwarded to {@link ConflictRound}.
 * @returns {Promise<{ results: object[], cancelled: boolean }>}
 */
export function resolveConflicts(conflicts, opts = {}) {
    const round = new ConflictRound(opts);
    for (const c of conflicts) round.enqueue(c);
    return round.run();
}
