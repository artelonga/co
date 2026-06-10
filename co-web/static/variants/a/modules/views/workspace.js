// ===== Sala (Workspace) view — CO-355 =====
// Launcher for the unified Sala surface (docs/architecture/sala-surface.md):
// lists templates, creates a sala from one, then navigates to the canvas at
// /u/{universe}/sala/{slug}. The canvas itself lives ONLY in
// co-web/static/shared/sala.html (CO-352) — this view must never grow one.

import { state } from '../state.js';
import { esc } from '../helpers.js';

let _showToast = () => {};

export function injectWorkspaceCallbacks(callbacks) {
    _showToast = callbacks.showToast;
}

// ─── Template picker modal ────────────────────────────────────────────────────

let _pickerResolve = null;

function openTemplatePicker(templates) {
    return new Promise((resolve) => {
        _pickerResolve = resolve;

        const overlay = document.createElement('div');
        overlay.id = 'workspace-picker-overlay';
        overlay.className = 'modal-overlay';
        overlay.innerHTML = `
            <div class="modal-box workspace-picker" role="dialog" aria-modal="true" aria-label="Nova Sala">
                <div class="modal-header">
                    <h2 class="modal-title">Nova Sala</h2>
                    <button class="modal-close" id="ws-picker-close" aria-label="Fechar">&times;</button>
                </div>
                <p class="ws-picker-hint">Escolha um modelo para iniciar:</p>
                <ul class="ws-template-list" id="ws-template-list">
                    ${templates.map(t => `
                        <li class="ws-template-item" data-slug="${esc(t.slug)}">
                            <div class="ws-template-name">${esc(t.name)}</div>
                            ${t.description ? `<div class="ws-template-desc">${esc(t.description)}</div>` : ''}
                            ${t.metadata?.estimated_minutes ? `<span class="ws-template-badge">${esc(String(t.metadata.estimated_minutes))} min</span>` : ''}
                        </li>
                    `).join('')}
                </ul>
                <div class="modal-actions">
                    <button class="btn btn-secondary" id="ws-picker-cancel">Cancelar</button>
                </div>
            </div>`;

        document.body.appendChild(overlay);

        overlay.querySelector('#ws-picker-close').addEventListener('click', () => closePicker(null));
        overlay.querySelector('#ws-picker-cancel').addEventListener('click', () => closePicker(null));
        overlay.addEventListener('click', (e) => { if (e.target === overlay) closePicker(null); });

        overlay.querySelectorAll('.ws-template-item').forEach(item => {
            item.addEventListener('click', () => closePicker(item.dataset.slug));
        });
    });
}

function closePicker(slug) {
    const overlay = document.getElementById('workspace-picker-overlay');
    if (overlay) overlay.remove();
    if (_pickerResolve) { _pickerResolve(slug); _pickerResolve = null; }
}

// ─── Main render ─────────────────────────────────────────────────────────────

export async function renderWorkspace() {
    const content = document.querySelector('#content');
    content.className = 'content workspace-view';

    const universeKey = state.currentUniverse || 'template';
    const salaUrl = `/u/${encodeURIComponent(universeKey)}/sala`;

    content.innerHTML = `
        <div class="workspace-toolbar">
            <button class="btn btn-primary" id="ws-new-btn">+ Nova Sala</button>
            <a class="btn btn-secondary" id="ws-open-default" href="${esc(salaUrl)}">Abrir sala padrão</a>
        </div>
        <div class="workspace-canvas-area" id="ws-canvas">
            <div class="workspace-empty-state">
                <span class="material-symbols-outlined" style="font-size:3rem;opacity:.3">hub</span>
                <p>A sala é o canvas espacial deste universo. Crie uma nova a partir de um modelo ou abra a sala padrão.</p>
            </div>
        </div>`;

    document.getElementById('ws-new-btn').addEventListener('click', () =>
        handleNewWorkspace(universeKey));
}

async function handleNewWorkspace(universeKey) {
    let templates = [{ slug: 'blank', name: 'Blank', description: 'Empty canvas — build from scratch.' }];

    try {
        const resp = await fetch(`/api/v1/universes/${encodeURIComponent(universeKey)}/workspace-templates`);
        if (resp.ok) templates = await resp.json();
    } catch {
        // fallback to blank
    }

    const slug = await openTemplatePicker(templates);
    if (!slug) return;

    const workspaceSlug = `sala-${Date.now()}`;
    try {
        const resp = await fetch(
            `/api/v1/universes/${encodeURIComponent(universeKey)}/workspaces/${encodeURIComponent(workspaceSlug)}/from-template/${encodeURIComponent(slug)}`,
            { method: 'POST' }
        );
        if (!resp.ok) {
            _showToast('Erro ao criar sala. Tente novamente.', 'error');
            return;
        }
        // Hand off to the unified Sala surface — the only canvas there is.
        window.location.href =
            `/u/${encodeURIComponent(universeKey)}/sala/${encodeURIComponent(workspaceSlug)}`;
    } catch {
        _showToast('Erro ao criar sala.', 'error');
    }
}
