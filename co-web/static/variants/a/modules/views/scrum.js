// ===== Scrum board view (CO-368) =====
// Renders the three-column Scrum board — Product Backlog / Sprint Backlog /
// Increment — for a universe whose `_scrum.yaml` declares `enabled: true`.
// The sprint goal sits above the columns; each PBI card shows its Definition-
// of-Done checklist inline, and ticking a box PATCHes the entry.
import { state } from '../state.js';
import { api } from '../api.js';

// CO-368: build the board via DOM (createElement + textContent) instead of
// innerHTML — textContent escapes by construction, so PBI titles/goals can't
// inject markup and the security scanner (CWE-79) stays clean.
function el(tag, className, opts = {}) {
    const e = document.createElement(tag);
    if (className) e.className = className;
    if (opts.text != null) e.textContent = opts.text;
    if (opts.i18n) e.dataset.i18n = opts.i18n;
    return e;
}

let _showToast = () => {};

export function injectScrumCallbacks(callbacks) {
    if (callbacks && callbacks.showToast) _showToast = callbacks.showToast;
}

// Cache the manifest so app.js can decide whether to show the Scrum tab
// without re-fetching on every render.
let _manifest = { enabled: false, current_sprint: null };

export function scrumEnabled() {
    return !!(_manifest && _manifest.enabled);
}

// Fetch + cache the manifest for the current universe. Call on universe load.
export async function refreshScrumManifest() {
    _manifest = await api.getScrumManifest();
    return _manifest;
}

// The three columns map onto PBI `status`.
const COLUMNS = [
    { key: 'backlog', i18n: 'scrum_product_backlog', label: 'Product Backlog', statuses: ['backlog', 'ready'] },
    { key: 'in-sprint', i18n: 'scrum_sprint_backlog', label: 'Sprint Backlog', statuses: ['in-sprint'] },
    { key: 'done', i18n: 'scrum_increment', label: 'Increment', statuses: ['done'] },
];

function pbiId(pbi) {
    // Path is `scrum/pbi/<id>.md`; the id is the stem.
    const p = pbi.path || '';
    const m = p.match(/([^/]+)\.md$/);
    return m ? m[1] : p;
}

function dodChecklist(pbi) {
    const fm = pbi.frontmatter || {};
    const dod = Array.isArray(fm.dod) ? fm.dod : [];
    if (!dod.length && Array.isArray(fm.acceptance)) {
        // Before any check-off, show acceptance criteria as an unchecked DoD.
        return fm.acceptance.map((text) => ({ text, done: false }));
    }
    return dod;
}

function renderCard(pbi) {
    const fm = pbi.frontmatter || {};
    const id = pbiId(pbi);
    const card = el('div', 'scrum-card');
    card.dataset.pbi = id;

    const head = el('div', 'scrum-card-head');
    if (fm.priority) {
        head.appendChild(el('span', `scrum-priority scrum-priority-${fm.priority}`, { text: fm.priority }));
    }
    if (fm.points != null) {
        head.appendChild(el('span', 'scrum-points', { text: String(fm.points) }));
    }
    card.appendChild(head);
    card.appendChild(el('div', 'scrum-card-title', { text: pbi.title || id }));

    const dod = dodChecklist(pbi);
    if (dod.length) {
        const ul = el('ul', 'scrum-dod');
        dod.forEach((item, i) => {
            const li = document.createElement('li');
            const label = document.createElement('label');
            const cb = el('input', 'scrum-dod-check');
            cb.type = 'checkbox';
            cb.dataset.pbi = id;
            cb.dataset.index = String(i);
            if (item.done) cb.checked = true;
            label.appendChild(cb);
            label.appendChild(document.createTextNode(' ' + (item.text ?? '')));
            li.appendChild(label);
            ul.appendChild(li);
        });
        card.appendChild(ul);
    }
    return card;
}

export async function renderScrum() {
    const content = document.querySelector('#content');
    content.className = 'content scrum-view';

    if (!scrumEnabled()) {
        content.replaceChildren(
            el('div', 'scrum-empty', {
                text: 'Scrum não está habilitado neste universo.',
                i18n: 'scrum_disabled',
            })
        );
        return;
    }

    content.replaceChildren();
    const sprint = _manifest.current_sprint;
    if (sprint) {
        const header = el('div', 'scrum-sprint-header');
        header.appendChild(el('span', 'scrum-sprint-number', { text: `Sprint ${sprint.number}` }));
        if (sprint.goal) {
            header.appendChild(el('span', 'scrum-sprint-goal', { text: sprint.goal }));
        }
        if (sprint.release_window) {
            header.appendChild(
                el('span', 'scrum-release-window', {
                    text: 'Release iminente',
                    i18n: 'scrum_release_window',
                })
            );
        }
        content.appendChild(header);
    }

    const board = el('div', 'scrum-board');
    const bodies = {};
    for (const c of COLUMNS) {
        const col = el('div', 'scrum-column');
        col.dataset.col = c.key;
        col.appendChild(el('h3', 'scrum-column-title', { text: c.label, i18n: c.i18n }));
        const body = el('div', 'scrum-column-body');
        body.id = `scrum-col-${c.key}`;
        body.appendChild(el('div', 'scrum-loading', { text: '…' }));
        col.appendChild(body);
        board.appendChild(col);
        bodies[c.key] = body;
    }
    content.appendChild(board);

    // Load the whole backlog once, then bucket by status client-side.
    let pbis = [];
    try {
        pbis = await api.getScrumBacklog();
    } catch (_) {
        pbis = [];
    }

    for (const col of COLUMNS) {
        const body = bodies[col.key];
        if (!body) continue;
        const items = pbis.filter((p) => col.statuses.includes((p.frontmatter || {}).status));
        if (items.length) {
            body.replaceChildren(...items.map(renderCard));
        } else {
            body.replaceChildren(el('div', 'scrum-column-empty', { text: '—' }));
        }
    }

    bindDodCheckboxes(content);
}

function bindDodCheckboxes(root) {
    root.querySelectorAll('.scrum-dod-check').forEach((box) => {
        box.addEventListener('change', async (e) => {
            const el = e.currentTarget;
            const id = el.dataset.pbi;
            const index = parseInt(el.dataset.index, 10);
            const done = el.checked;
            try {
                await api.checkPbiDod(id, index, done);
                _showToast('DoD atualizado');
            } catch (err) {
                el.checked = !done; // revert on failure
                _showToast('Falha ao atualizar DoD');
            }
        });
    });
}
