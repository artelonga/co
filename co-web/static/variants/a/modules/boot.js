// ===== Universe boot sequence =====
import { state } from './state.js';
import { api, apiFetch } from './api.js';
import { esc } from './helpers.js';
import { withTimeout, loadThemeCss, applyUniverseConfig, renderSettingsGear } from './settings.js';
import { renderUsageCount } from './sidebar.js';

let _showLoading = () => {};
let _hideLoading = () => {};
let _render = () => {};
let _selectProject = async () => {};
let _removeManifestViewTabs = () => {};
let _injectManifestViewTabs = () => {};
let _switchView = () => {};
// CO-383: called after universe info is loaded so the caller can show/hide
// read-only banners based on source_kind.
let _onUniverseInfoLoaded = (_info) => {};

export function injectBootCallbacks(callbacks) {
    _showLoading = callbacks.showLoading;
    _hideLoading = callbacks.hideLoading;
    _render = callbacks.render;
    _selectProject = callbacks.selectProject;
    _removeManifestViewTabs = callbacks.removeManifestViewTabs;
    _injectManifestViewTabs = callbacks.injectManifestViewTabs;
    _switchView = callbacks.switchView;
    if (callbacks.onUniverseInfoLoaded) _onUniverseInfoLoaded = callbacks.onUniverseInfoLoaded;
}

// Universe home — fallback when no project exists
export async function renderUniverseHome() {
    const content = document.querySelector('#content');
    if (!content) return;
    const slug = state.currentUniverseSlug;
    const info = state.universeInfo || {};
    const name = info.name || slug || '';
    const description = info.description || '';

    content.className = 'content universe-home-view';
    content.innerHTML = `
        <div class="universe-home">
            <header class="universe-home-header">
                <h1 class="universe-home-title">${esc(name)}</h1>
                ${description ? `<p class="universe-home-desc">${esc(description)}</p>` : ''}
            </header>
            <div class="universe-home-body" id="universe-home-body">
                <div class="universe-home-loading">Carregando…</div>
            </div>
        </div>`;

    let indexEntry = null;
    try {
        indexEntry = await apiFetch(
            `/api/v1/universes/${encodeURIComponent(slug)}/entries/${encodeURIComponent('index.md')}`,
            {}, true
        );
    } catch (_) {}

    const body = document.querySelector('#universe-home-body');
    if (!body) return;

    if (indexEntry && indexEntry.body) {
        const md = window.CoMarkdown;
        const html = md ? md.renderMarkdown(indexEntry.body) : esc(indexEntry.body);
        body.innerHTML = `<article class="universe-home-md md-body">${html}</article>`;
        if (md && typeof md.renderMermaidBlocks === 'function') {
            md.renderMermaidBlocks(body);
        }
        // Inject "Ver mais" — top entries by popularity, replacing the sentinel comment.
        try {
            const popular = await apiFetch(
                `/api/v1/universes/${encodeURIComponent(slug)}/entries/popular?limit=5`,
                {}, true
            );
            if (popular && popular.length > 0) {
                const listHtml = popular
                    .map(e => `<li><a href="/${esc(slug)}?path=${encodeURIComponent(e.path)}">${esc(e.title)}</a></li>`)
                    .join('');
                const section = document.createElement('section');
                section.className = 'universe-home-popular';
                section.innerHTML = `<h2>Ver mais</h2><ul class="universe-popular-list">${listHtml}</ul>`;
                const article = body.querySelector('article');
                if (article) article.appendChild(section);
            }
        } catch (_) {}
        return;
    }

    const totalEntries = (info.content_count || state.projects?.length || 0);
    body.innerHTML = `
        <div class="universe-home-empty">
            <p>Este universo ainda não tem uma página inicial.</p>
            <p style="color:var(--text-muted,#6b7280);font-size:13px">
                Crie um arquivo <code>index.md</code> na raiz do universo para descrever
                o que ele é. Ele será renderizado aqui.
            </p>
            ${totalEntries ? `<p style="color:var(--text-muted,#6b7280);font-size:13px">Há <strong>${totalEntries}</strong> entrada(s) neste universo. Acesse pelo menu lateral ou pela visão Conteúdo.</p>` : ''}
        </div>`;
}

export async function bootApp(getProjects, selectProject) {
    _showLoading();
    state.projects = await api.getProjects();
    if (state.projects.length > 0) {
        await _selectProject(state.projects[0].key);
    }
    _hideLoading();
    _render();
}

export async function bootAppForUniverse(slug) {
    state.switchingUniverse = true;
    state.tasks = [];
    state.projects = [];
    state.currentProject = null;
    state.universeInfo = null;
    state.universeConfig = null;
    state.universeManifest = null;
    state.calendarEntries = [];
    _removeManifestViewTabs();

    _showLoading();

    let watchdogFired = false;
    const watchdogTimer = setTimeout(() => {
        watchdogFired = true;
        state.switchingUniverse = false;
        const content = document.querySelector('#content');
        if (content) {
            content.innerHTML = `
                <div style="padding:24px;max-width:520px;margin:40px auto;background:var(--card-bg,#fff);border:1px solid var(--border,#e5e7eb);border-radius:8px">
                    <h2 style="margin:0 0 12px;font-size:18px">Carregamento demorou demais</h2>
                    <p style="margin:0 0 12px;color:var(--text-muted,#6b7280)">
                        Não conseguimos carregar <strong>${esc(slug)}</strong> em 20s.
                    </p>
                    <p style="margin:0;display:flex;gap:8px;flex-wrap:wrap">
                        <button class="btn btn-primary btn-sm" onclick="window.location.reload()">Recarregar</button>
                        <a class="btn btn-secondary btn-sm" href="/" style="text-decoration:none">Voltar ao template</a>
                        <a class="btn btn-secondary btn-sm" href="/reset-sw.html" style="text-decoration:none">Reset cache</a>
                    </p>
                </div>`;
        }
    }, 20000);

    try {
        let info = null;
        let config = null;
        try {
            const [i, c, m] = await Promise.all([
                withTimeout(api.getUniverseInfo(slug), 8000, 'getUniverseInfo'),
                withTimeout(api.getUniverseConfig(slug), 8000, 'getUniverseConfig'),
                withTimeout(api.getUniverseManifest(slug), 8000, 'getUniverseManifest'),
            ]);
            info = i; config = c;
            if (m) {
                state.universeManifest = m;
                _injectManifestViewTabs(m);
            }
        } catch (e) {
            console.warn('bootAppForUniverse: info/config fetch failed', e);
        }

        if (watchdogFired) return;

        if (info) {
            state.universeInfo = info;
            renderUsageCount();
            _onUniverseInfoLoaded(info);
        }
        if (config) {
            applyUniverseConfig(config);
        } else {
            loadThemeCss(slug);
        }
        renderSettingsGear(info);

        const projects = await withTimeout(api.getUniverseProjects(slug), 8000, 'getUniverseProjects');
        if (watchdogFired) return;
        state.projects = Array.isArray(projects) ? projects : [];

        if (state.projects.length > 0) {
            try {
                await withTimeout(_selectProject(state.projects[0].key), 8000, 'selectProject');
            } catch (e) {
                console.warn('bootAppForUniverse: selectProject threw', e);
            }
        }
    } finally {
        clearTimeout(watchdogTimer);
        if (!watchdogFired) {
            state.switchingUniverse = false;
            _hideLoading();
            try { _render(); } catch (e) { console.warn('bootAppForUniverse: render threw', e); }
        }
    }
}
