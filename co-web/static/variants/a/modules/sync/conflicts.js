// CO-385: Mac-style conflict resolution UI for cross-device sync.
//
// Renders a conflict list panel with 7 action buttons per row.
// Supports bulk-apply (sticky checkbox) to propagate one action to all
// remaining conflicts of the same shape.

import { apiFetch } from '../api/client.js';

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

export async function listConflicts(universe) {
    const url = universe
        ? `/api/v1/me/sync/conflicts?universe=${encodeURIComponent(universe)}`
        : '/api/v1/me/sync/conflicts';
    return apiFetch(url, {}, true);
}

export async function resolveConflict(conflictId, action, applyToAllMatching = false) {
    return apiFetch(`/api/v1/sync/conflicts/${encodeURIComponent(conflictId)}/resolve`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action, apply_to_all_matching: applyToAllMatching }),
    }, true);
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

const ACTION_LABELS = {
    keep_both:     'Manter ambos',
    ignore:        'Ignorar',
    replace:       'Substituir ←',
    update:        'Mesclar',
    upsert:        'Upsert',
    accept_delete: 'Aceitar exclusão',
    keep_local:    'Manter local',
};

// Actions available per ConflictKind
const ACTIONS_FOR_KIND = {
    both_modified:                  ['keep_both', 'replace', 'update', 'upsert', 'ignore'],
    local_only_new:                 ['keep_local', 'accept_delete', 'ignore'],
    remote_only_new:                ['replace', 'keep_both', 'ignore'],
    local_deleted_remote_modified:  ['accept_delete', 'keep_local', 'replace', 'ignore'],
    local_modified_remote_deleted:  ['keep_local', 'accept_delete', 'update', 'ignore'],
};

/**
 * Render the conflicts panel into `container`.
 *
 * @param {HTMLElement} container
 * @param {string|null} universe  Optional universe filter.
 */
export async function renderConflictsPanel(container, universe = null) {
    container.innerHTML = '<p class="co-loading">A carregar conflitos…</p>';

    const data = await listConflicts(universe);
    if (!data || !data.conflicts) {
        container.innerHTML = '<p class="co-error">Erro ao carregar conflitos.</p>';
        return;
    }

    const conflicts = data.conflicts;
    if (conflicts.length === 0) {
        container.innerHTML = '<p class="co-empty">Nenhum conflito pendente.</p>';
        return;
    }

    let bulkAction = null;

    const panel = document.createElement('div');
    panel.className = 'co-conflicts-panel';

    // Bulk-apply checkbox
    const bulkRow = document.createElement('div');
    bulkRow.className = 'co-conflicts-bulk';
    const bulkCheckbox = document.createElement('input');
    bulkCheckbox.type = 'checkbox';
    bulkCheckbox.id = 'co-bulk-apply';
    const bulkLabel = document.createElement('label');
    bulkLabel.htmlFor = 'co-bulk-apply';
    bulkLabel.textContent = 'Aplicar esta ação a todos os conflitos do mesmo tipo';
    bulkRow.appendChild(bulkCheckbox);
    bulkRow.appendChild(bulkLabel);
    panel.appendChild(bulkRow);

    // Conflict count header
    const header = document.createElement('h3');
    header.className = 'co-conflicts-header';
    header.textContent = `${conflicts.length} conflito(s) pendente(s)`;
    panel.appendChild(header);

    // Render each conflict row
    for (const conflict of conflicts) {
        const row = renderConflictRow(conflict, async (action) => {
            const applyToAll = bulkCheckbox.checked;
            const result = await resolveConflict(conflict.id, action, applyToAll);
            if (result) {
                // Reload panel
                await renderConflictsPanel(container, universe);
            }
        });
        panel.appendChild(row);
    }

    container.innerHTML = '';
    container.appendChild(panel);
}

/**
 * Render a single conflict row with applicable action buttons.
 *
 * @param {object} conflict
 * @param {function} onAction  Called with the chosen action string.
 * @returns {HTMLElement}
 */
function renderConflictRow(conflict, onAction) {
    const row = document.createElement('div');
    row.className = 'co-conflict-row';
    row.dataset.conflictId = conflict.id;
    row.dataset.kind = conflict.kind;

    // Path header
    const pathEl = document.createElement('div');
    pathEl.className = 'co-conflict-path';
    pathEl.textContent = `📝 ${conflict.universe_key}::${conflict.path}`;
    row.appendChild(pathEl);

    // Hash diff info
    const hashEl = document.createElement('div');
    hashEl.className = 'co-conflict-hashes';
    hashEl.innerHTML = `
        <span class="co-hash-local" title="Hash local">💻 ${conflict.local_body_hash.slice(0, 8)}…</span>
        <span class="co-hash-remote" title="Hash remoto">📱 ${conflict.remote_body_hash.slice(0, 8)}…</span>
    `;
    row.appendChild(hashEl);

    // Suggested action badge
    if (conflict.suggested_action) {
        const suggEl = document.createElement('span');
        suggEl.className = 'co-suggested-action';
        suggEl.textContent = `Sugerido: ${ACTION_LABELS[conflict.suggested_action] || conflict.suggested_action}`;
        row.appendChild(suggEl);
    }

    // Action buttons
    const actionsEl = document.createElement('div');
    actionsEl.className = 'co-conflict-actions';

    const applicableActions = ACTIONS_FOR_KIND[conflict.kind] || ['update', 'replace', 'ignore'];
    for (const action of applicableActions) {
        const btn = document.createElement('button');
        btn.className = `co-conflict-btn co-action-${action}`;
        btn.textContent = ACTION_LABELS[action] || action;
        btn.addEventListener('click', () => {
            // Disable all buttons while resolving
            actionsEl.querySelectorAll('button').forEach(b => { b.disabled = true; });
            row.classList.add('co-resolving');
            onAction(action).catch(() => {
                actionsEl.querySelectorAll('button').forEach(b => { b.disabled = false; });
                row.classList.remove('co-resolving');
            });
        });
        actionsEl.appendChild(btn);
    }
    row.appendChild(actionsEl);

    return row;
}

// ---------------------------------------------------------------------------
// Live event integration (CO-381 /agora)
// ---------------------------------------------------------------------------

/**
 * Wire up the live timeline to show a "Resolver →" CTA when a
 * `sync.conflict_detected` event arrives on the EDA WebSocket.
 *
 * @param {EventTarget} eventBus  The CO EDA event emitter (from events_ws.js).
 * @param {function} openPanel    Called when the user clicks "Resolver →".
 */
export function wireLiveConflictCta(eventBus, openPanel) {
    eventBus.addEventListener('sync.conflict_detected', (ev) => {
        const { universe, path, conflict_id, resolve_url } = ev.detail || {};
        if (!conflict_id) return;

        const toast = document.createElement('div');
        toast.className = 'co-conflict-toast';
        toast.innerHTML = `
            <strong>Conflito detectado</strong>:
            <span class="co-conflict-toast-path">${universe}::${path}</span>
            <a href="${resolve_url || '/settings/sync'}" class="co-conflict-toast-cta">
                Resolver →
            </a>
        `;
        toast.querySelector('.co-conflict-toast-cta').addEventListener('click', (e) => {
            e.preventDefault();
            openPanel(universe);
        });

        document.body.appendChild(toast);
        setTimeout(() => toast.remove(), 8000);
    });
}
