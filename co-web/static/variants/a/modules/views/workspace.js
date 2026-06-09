// ===== Sala (Workspace) view — CO-352 =====
// Launcher for the unified Sala surface (docs/architecture/sala-surface.md):
// one surface, fractal scope. The canvas lives ONLY in
// co-web/static/shared/sala.html, served at /u/{universe}/sala[/{slug}].
// This SPA view never grows its own canvas — it only explains and opens.
// CO-355 extends this launcher with the workspace-template picker.

import { state } from '../state.js';

let _showToast = () => {};

export function injectWorkspaceCallbacks(callbacks) {
    _showToast = callbacks.showToast || _showToast;
}

function esc(s) {
    return String(s || '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

export function renderWorkspace() {
    const content = document.querySelector('#content');
    content.className = 'content workspace-view';

    const universeKey = state.currentUniverse || 'template';
    const salaUrl = `/u/${encodeURIComponent(universeKey)}/sala`;

    content.innerHTML = `
        <div class="workspace-toolbar">
            <a class="btn btn-primary" id="ws-open-sala" href="${esc(salaUrl)}">Abrir Sala</a>
        </div>
        <div class="workspace-canvas-area" id="ws-canvas">
            <div class="workspace-empty-state">
                <span class="material-symbols-outlined" style="font-size:3rem;opacity:.3">hub</span>
                <p>A sala é o canvas espacial deste universo — arraste entradas,
                   ligue ideias, compartilhe a vista. Uma única superfície,
                   do universo inteiro a qualquer recorte.</p>
            </div>
        </div>`;
}
