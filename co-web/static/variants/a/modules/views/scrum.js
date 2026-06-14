// ===== Scrum board view (CO-368) =====
// Renders the three-column Scrum board — Product Backlog / Sprint Backlog /
// Increment — for a universe whose `_scrum.yaml` declares `enabled: true`.
// The sprint goal sits above the columns; each PBI card shows its Definition-
// of-Done checklist inline, and ticking a box PATCHes the entry.
import { state } from '../state.js';
import { api } from '../api.js';
import { esc } from '../helpers.js';

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
    const points = fm.points != null ? `<span class="scrum-points">${esc(String(fm.points))}</span>` : '';
    const priority = fm.priority ? `<span class="scrum-priority scrum-priority-${esc(fm.priority)}">${esc(fm.priority)}</span>` : '';
    const dod = dodChecklist(pbi);
    const dodHtml = dod.length
        ? `<ul class="scrum-dod">${dod
              .map(
                  (item, i) =>
                      `<li><label><input type="checkbox" class="scrum-dod-check" data-pbi="${esc(id)}" data-index="${i}" ${item.done ? 'checked' : ''}> ${esc(item.text)}</label></li>`
              )
              .join('')}</ul>`
        : '';
    return `
        <div class="scrum-card" data-pbi="${esc(id)}">
            <div class="scrum-card-head">${priority}${points}</div>
            <div class="scrum-card-title">${esc(pbi.title || id)}</div>
            ${dodHtml}
        </div>`;
}

export async function renderScrum() {
    const content = document.querySelector('#content');
    content.className = 'content scrum-view';

    if (!scrumEnabled()) {
        content.innerHTML = `<div class="scrum-empty" data-i18n="scrum_disabled">Scrum não está habilitado neste universo.</div>`;
        return;
    }

    const sprint = _manifest.current_sprint;
    const goal = sprint && sprint.goal ? sprint.goal : '';
    const sprintHeader = sprint
        ? `<div class="scrum-sprint-header">
               <span class="scrum-sprint-number">Sprint ${esc(String(sprint.number))}</span>
               ${goal ? `<span class="scrum-sprint-goal">${esc(goal)}</span>` : ''}
               ${sprint.release_window ? `<span class="scrum-release-window" data-i18n="scrum_release_window">Release iminente</span>` : ''}
           </div>`
        : '';

    content.innerHTML = `
        ${sprintHeader}
        <div class="scrum-board">
            ${COLUMNS.map(
                (c) =>
                    `<div class="scrum-column" data-col="${c.key}">
                        <h3 class="scrum-column-title" data-i18n="${c.i18n}">${esc(c.label)}</h3>
                        <div class="scrum-column-body" id="scrum-col-${c.key}"><div class="scrum-loading">…</div></div>
                    </div>`
            ).join('')}
        </div>`;

    // Load the whole backlog once, then bucket by status client-side.
    let pbis = [];
    try {
        pbis = await api.getScrumBacklog();
    } catch (_) {
        pbis = [];
    }

    for (const col of COLUMNS) {
        const body = document.querySelector(`#scrum-col-${col.key}`);
        if (!body) continue;
        const items = pbis.filter((p) => col.statuses.includes((p.frontmatter || {}).status));
        body.innerHTML = items.length
            ? items.map(renderCard).join('')
            : `<div class="scrum-column-empty">—</div>`;
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
