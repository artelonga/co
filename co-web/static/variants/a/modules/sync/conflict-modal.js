// CO-128: Apple-Finder-style 4-way conflict-resolution modal.
//
// Surfaces the four conflict actions the user named as a v1 requirement when a
// sync detects divergent edits on the same entry:
//
//   1 = Ignore      keep the remote version, discard local
//   2 = Replace     keep the local version, discard remote
//   3 = Keep both   persist both (remote stays, local becomes `<name>.local.md`)
//   Esc = cancel    abort the whole sync round
//
//   "Apply to all"  (checkbox) — batch the chosen action across the remaining
//                   conflicts queued in the current sync round.
//
// The component is pure UI: it consumes a `ConflictPayload { local, remote,
// base? }`, renders a monospace side-by-side diff, and reports the choice back
// through `onResolve` / the returned promise. The round controller
// (conflict-round.js) wires the choice to the real entry API.

import { lineDiff } from './conflict-diff.js';

// Visible action labels — resolved through window.t when available, with a
// Portuguese-first fallback (PT-BR is the primary locale).
function label(key, fallback) {
    return (typeof window !== 'undefined' && typeof window.t === 'function')
        ? window.t(key)
        : fallback;
}

// Inject the modal's scoped styles once. Uses CO design tokens (CSS custom
// properties) so it inherits the active theme — no new framework, no per-variant
// stylesheet edits.
let _stylesInjected = false;
function injectStyles() {
    if (_stylesInjected) return;
    if (typeof document === 'undefined') return;
    _stylesInjected = true;
    const style = document.createElement('style');
    style.id = 'co-conflict-modal-styles';
    style.textContent = `
.co-conflict-overlay {
    position: fixed; inset: 0; z-index: 1000;
    display: flex; align-items: center; justify-content: center;
    background: var(--modal-overlay, rgba(0,0,0,0.4));
}
.co-conflict-modal {
    background: var(--modal-surface, var(--card-bg, #fff));
    color: var(--text-primary, #111827);
    border-radius: var(--radius-lg, 12px);
    box-shadow: var(--shadow-lg, 0 10px 15px -3px rgba(0,0,0,0.2));
    width: min(720px, 92vw); max-height: 86vh;
    display: flex; flex-direction: column;
    font-family: var(--font, -apple-system, system-ui, sans-serif);
}
.co-conflict-head { display: flex; gap: 14px; padding: 20px 22px 8px; align-items: flex-start; }
.co-conflict-icon { font-size: 34px; line-height: 1; flex: none; }
.co-conflict-titles { flex: 1 1 auto; min-width: 0; }
.co-conflict-title { font-size: 15px; font-weight: 600; margin: 0 0 4px; }
.co-conflict-subtitle { font-size: 13px; color: var(--text-secondary, #6b7280); margin: 0; word-break: break-all; }
.co-conflict-remaining { font-size: 12px; color: var(--text-secondary, #6b7280); margin: 4px 0 0; }
.co-conflict-diff {
    margin: 12px 22px; border: 1px solid var(--border, #e5e7eb);
    border-radius: var(--radius-md, 8px); overflow: auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12px; line-height: 1.5; background: var(--bg, #f8f9fb);
}
.co-conflict-diff table { width: 100%; border-collapse: collapse; }
.co-conflict-diff th {
    position: sticky; top: 0; text-align: left; padding: 6px 10px;
    background: var(--bg-hover, #eceef1); color: var(--text-secondary, #6b7280);
    font-family: var(--font, system-ui); font-size: 11px; font-weight: 600;
    width: 50%; border-bottom: 1px solid var(--border, #e5e7eb);
}
.co-conflict-diff td {
    padding: 1px 10px; white-space: pre-wrap; word-break: break-word;
    vertical-align: top; width: 50%;
    border-right: 1px solid var(--border, #e5e7eb);
}
.co-conflict-diff tr.del td.l { background: rgba(239,68,68,0.12); }
.co-conflict-diff tr.add td.r { background: rgba(34,197,94,0.14); }
.co-conflict-diff td.empty { background: var(--bg-hover, #f1f2f4); }
.co-conflict-truncated { padding: 8px 22px 0; font-size: 12px; color: var(--danger, #ef4444); }
.co-conflict-applyall { display: flex; align-items: center; gap: 8px; padding: 10px 22px 4px; font-size: 13px; }
.co-conflict-actions {
    display: flex; gap: 8px; justify-content: flex-end;
    padding: 14px 22px 20px; flex-wrap: wrap;
}
.co-conflict-btn {
    padding: 8px 16px; border-radius: var(--radius-md, 8px); cursor: pointer;
    font-size: 13px; font-weight: 600; border: 1px solid var(--border, #e5e7eb);
    background: var(--card-bg, #fff); color: var(--text-primary, #111827);
}
.co-conflict-btn:hover { background: var(--bg-hover, #eee); }
.co-conflict-btn .k { opacity: 0.55; font-weight: 500; margin-left: 6px; font-size: 11px; }
.co-conflict-btn-primary { background: var(--accent, #6366f1); border-color: var(--accent, #6366f1); color: #fff; }
.co-conflict-btn-primary:hover { background: var(--accent-hover, #4f46e5); }
.co-conflict-btn-danger { color: var(--danger, #ef4444); }
`;
    document.head.appendChild(style);
}

function renderDiff(payload) {
    const wrap = document.createElement('div');
    wrap.className = 'co-conflict-diff';

    const { rows, truncated } = lineDiff(payload.local?.body, payload.remote?.body);

    // CO-128: built via DOM + textContent (not innerHTML) — textContent
    // escapes by construction, so user-supplied entry bodies can never inject
    // markup, and the security scanner stays clean.
    const table = document.createElement('table');
    const thead = document.createElement('thead');
    const headRow = document.createElement('tr');
    const thLocal = document.createElement('th');
    thLocal.textContent = label('conflict.col_local', 'Sua versão (local)');
    const thRemote = document.createElement('th');
    thRemote.textContent = label('conflict.col_remote', 'Versão do servidor (remoto)');
    headRow.append(thLocal, thRemote);
    thead.appendChild(headRow);
    table.appendChild(thead);

    const tbody = document.createElement('tbody');
    const mkCell = (val, cls) => {
        const td = document.createElement('td');
        td.className = val === null ? `${cls} empty` : cls;
        if (val !== null) td.textContent = val;
        return td;
    };
    for (const row of rows) {
        const tr = document.createElement('tr');
        tr.className = row.kind;
        tr.append(mkCell(row.left, 'l'), mkCell(row.right, 'r'));
        tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    wrap.appendChild(table);

    if (truncated || payload.local?.truncated || payload.remote?.truncated) {
        wrap.dataset.truncated = 'true';
    }
    return wrap;
}

/**
 * Build (but do not yet display) the conflict modal for a single conflict.
 *
 * @param {object} payload  ConflictPayload { universe_key, path, local, remote, base? }
 * @param {object} opts
 * @param {boolean} [opts.applyToAllChecked=false]  Initial checkbox state.
 * @param {number}  [opts.remaining=0]  Conflicts still queued after this one.
 * @param {(action: string, applyToAll: boolean) => void} opts.onResolve
 *        Called once with the chosen action ('ignore'|'replace'|'keep_both'|'cancel').
 * @returns {{ overlay: HTMLElement, destroy: () => void }}
 */
export function buildConflictModal(payload, opts = {}) {
    injectStyles();
    const remaining = opts.remaining || 0;
    let resolved = false;

    const overlay = document.createElement('div');
    overlay.className = 'co-conflict-overlay';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.dataset.conflictPath = payload.path || '';

    const modal = document.createElement('div');
    modal.className = 'co-conflict-modal';
    overlay.appendChild(modal);

    // Header: warning icon + title + entry path (Finder-style).
    const head = document.createElement('div');
    head.className = 'co-conflict-head';
    const icon = document.createElement('div');
    icon.className = 'co-conflict-icon';
    icon.setAttribute('aria-hidden', 'true');
    icon.textContent = '⚠️';
    const titles = document.createElement('div');
    titles.className = 'co-conflict-titles';
    const mkP = (cls, text) => {
        const p = document.createElement('p');
        p.className = cls;
        p.textContent = text;
        return p;
    };
    titles.appendChild(mkP('co-conflict-title', label('conflict.title', 'Conflito de edição')));
    titles.appendChild(mkP('co-conflict-subtitle', payload.path || ''));
    titles.appendChild(mkP('co-conflict-subtitle',
        label('conflict.subtitle', 'Esta entrada foi alterada em outro lugar. Escolha como resolver:')));
    if (remaining > 0) {
        titles.appendChild(mkP('co-conflict-remaining',
            label('conflict.remaining', '+ {n} conflito(s) restante(s) nesta rodada').replace('{n}', String(remaining))));
    }
    head.append(icon, titles);
    modal.appendChild(head);

    // Side-by-side diff.
    const diffEl = renderDiff(payload);
    modal.appendChild(diffEl);
    if (diffEl.dataset.truncated === 'true') {
        const tr = document.createElement('p');
        tr.className = 'co-conflict-truncated';
        tr.textContent = label('conflict.truncated', 'Conteúdo muito grande — pré-visualização truncada.');
        modal.appendChild(tr);
    }

    // "Apply to all" checkbox.
    const applyRow = document.createElement('label');
    applyRow.className = 'co-conflict-applyall';
    const applyBox = document.createElement('input');
    applyBox.type = 'checkbox';
    applyBox.className = 'co-conflict-applyall-box';
    applyBox.checked = !!opts.applyToAllChecked;
    const applyText = document.createElement('span');
    applyText.textContent = label('conflict.apply_to_all', 'Aplicar a todos os conflitos desta rodada');
    applyRow.appendChild(applyBox);
    applyRow.appendChild(applyText);
    modal.appendChild(applyRow);

    // Action buttons. Finder order (left→right): Keep both · Ignore · Replace.
    const actions = document.createElement('div');
    actions.className = 'co-conflict-actions';

    function finish(action) {
        if (resolved) return;
        resolved = true;
        document.removeEventListener('keydown', onKey, true);
        if (typeof opts.onResolve === 'function') {
            opts.onResolve(action, applyBox.checked);
        }
    }

    const btnKeepBoth = mkBtn('co-conflict-btn', label('conflict.keep_both', 'Manter ambos'), '3', () => finish('keep_both'));
    const btnIgnore = mkBtn('co-conflict-btn co-conflict-btn-danger', label('conflict.ignore', 'Ignorar'), '1', () => finish('ignore'));
    const btnReplace = mkBtn('co-conflict-btn co-conflict-btn-primary', label('conflict.replace', 'Substituir'), '2', () => finish('replace'));
    actions.appendChild(btnKeepBoth);
    actions.appendChild(btnIgnore);
    actions.appendChild(btnReplace);
    modal.appendChild(actions);

    // Keyboard shortcuts: 1=Ignore, 2=Replace, 3=Keep both, Esc=cancel round.
    function onKey(e) {
        // Ignore shortcuts while typing in an input/textarea elsewhere.
        const tag = (e.target && e.target.tagName) || '';
        const typing = tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT';
        if (e.key === 'Escape') {
            e.preventDefault();
            finish('cancel');
            return;
        }
        if (typing) return;
        if (e.key === '1') { e.preventDefault(); finish('ignore'); }
        else if (e.key === '2') { e.preventDefault(); finish('replace'); }
        else if (e.key === '3') { e.preventDefault(); finish('keep_both'); }
    }
    document.addEventListener('keydown', onKey, true);

    function destroy() {
        document.removeEventListener('keydown', onKey, true);
        overlay.remove();
    }

    return { overlay, destroy };
}

function mkBtn(cls, text, keyHint, onClick) {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = cls;
    b.appendChild(document.createTextNode(text));
    if (keyHint) {
        const k = document.createElement('span');
        k.className = 'k';
        k.textContent = keyHint;
        b.appendChild(k);
    }
    b.addEventListener('click', onClick);
    return b;
}

/**
 * Open the conflict modal and resolve with the user's choice.
 *
 * @param {object} payload  ConflictPayload { universe_key, path, local, remote, base? }
 * @param {object} [opts]   { applyToAllChecked, remaining }
 * @returns {Promise<{ action: string, applyToAll: boolean }>}
 */
export function openConflictModal(payload, opts = {}) {
    return new Promise((resolve) => {
        const { overlay, destroy } = buildConflictModal(payload, {
            ...opts,
            onResolve: (action, applyToAll) => {
                destroy();
                resolve({ action, applyToAll });
            },
        });
        document.body.appendChild(overlay);
        // Focus the modal so keyboard shortcuts work without a click first.
        overlay.querySelector('.co-conflict-btn-primary')?.focus();
    });
}
