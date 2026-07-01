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

// CO-98: build the "Explorar este universo" panel listing a universe's direct
// children (hierarchical universes). Fetches the anonymous-safe public list and
// filters by `parent_key === slug`. Returns '' when there are no children, so
// callers can append unconditionally. Each card links to the child at `/<slug>`,
// matching the SPA's universe URL scheme (see setUniverseSlugInUrl).
// Returns an HTMLElement (the panel) or null when there are no children.
// Built via DOM APIs (textContent) — XSS-safe by construction, no innerHTML.
async function buildExplorarPanel(slug) {
    let universes = [];
    try {
        universes = await apiFetch('/api/v1/universes/public', {}, true) || [];
    } catch (_) {
        return null;
    }
    const children = universes
        .filter(u => u.parent_key === slug)
        .sort((a, b) => (a.name || a.key).localeCompare(b.name || b.key));
    if (children.length === 0) return null;

    const section = document.createElement('section');
    section.className = 'universe-home-explorar';
    section.setAttribute('aria-label', 'Explorar este universo');
    const h2 = document.createElement('h2');
    h2.className = 'explorar-title';
    h2.textContent = 'Explorar este universo';
    const cardsWrap = document.createElement('div');
    cardsWrap.className = 'explorar-cards';
    for (const c of children) {
        const a = document.createElement('a');
        a.className = 'explorar-card';
        a.href = `/${encodeURIComponent(c.key)}`;
        const nameEl = document.createElement('span');
        nameEl.className = 'explorar-card-name';
        nameEl.textContent = c.name || c.key;
        a.appendChild(nameEl);
        if (c.description) {
            const descEl = document.createElement('span');
            descEl.className = 'explorar-card-desc';
            descEl.textContent = c.description;
            a.appendChild(descEl);
        }
        cardsWrap.appendChild(a);
    }
    section.appendChild(h2);
    section.appendChild(cardsWrap);
    return section;
}

// Universe home — fallback when no project exists
export async function renderUniverseHome() {
    const content = document.querySelector('#content');
    if (!content) return;
    const slug = state.currentUniverseSlug;
    const info = state.universeInfo || {};
    const name = info.name || slug || '';
    const description = info.description || '';
    // CO-98: parent universes (e.g. the template hosting the timeline trio) get
    // an "Explorar" panel listing their direct children. Built once here and
    // injected into whichever home layout renders below (markdown or empty).
    const isParent = info.is_template === true || slug === 'template';
    const explorarPanel = isParent ? await buildExplorarPanel(slug) : null;

    // CO-555: drop any stale Conteúdo detail controller left on #content by a
    // prior conteudo render. Without this a language toggle could fan out to BOTH
    // retranslateConteudoDetail (via the leftover controller) and
    // retranslateUniverseHome, doubling the /api/ request. The home view owns the
    // node now, so a single twin fetch per toggle is guaranteed.
    delete content._detailController;
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
        // CO-555: the index markdown lives in its own swappable wrapper so a
        // language toggle can re-render just this body from the `en/index.md`
        // twin (retranslateUniverseHome) without re-running the whole home
        // fan-out. `data-index-path` records which twin is currently displayed.
        body.dataset.indexPath = 'index.md';
        body.innerHTML = `<article class="universe-home-md md-body"><div class="universe-home-index-body">${html}</div></article>`;
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
        // CO-98: append the Explorar panel below the rendered index page.
        if (explorarPanel) body.appendChild(explorarPanel);
        return;
    }

    // CO-98: on a parent universe with children, the empty-state hint is
    // replaced by the Explorar panel so the home page is never blank.
    if (explorarPanel) {
        body.replaceChildren(explorarPanel);
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

// CO-555: re-render the board-root home/index body in the target language.
//
// At the board root (no selected project) the universe index is rendered by
// renderUniverseHome — NOT by the Conteúdo detail controller — so
// retranslateConteudoDetail finds no current entry and no-ops, leaving the body
// in Portuguese after a toggle. This swaps just the index markdown for its
// language twin (`en/index.md` when EN, base `index.md` when PT) via a SINGLE
// raw fetch (silent on 404, like the detail-page path). At most one /api/
// request per toggle, never the renderUniverseHome fan-out. If the twin is
// missing the current language is kept gracefully — no popup.
export async function retranslateUniverseHome(targetLang) {
    const content = document.querySelector('#content');
    if (!content || !content.classList.contains('universe-home-view')) return;
    const body = content.querySelector('#universe-home-body');
    if (!body) return;
    const indexEl = body.querySelector('.universe-home-index-body');
    // No swappable index body → empty-state home or still loading; nothing to do.
    if (!indexEl) return;

    const slug = state.currentUniverseSlug;
    if (!slug) return;

    const curPath = body.dataset.indexPath || 'index.md';
    const EN_PREFIX = 'en/';
    const isEnPath = curPath.startsWith(EN_PREFIX);
    let twinPath;
    if (targetLang === 'en') {
        twinPath = isEnPath ? curPath : EN_PREFIX + curPath;
    } else {
        twinPath = isEnPath ? curPath.slice(EN_PREFIX.length) : curPath;
    }
    // Already displaying the target language → no fetch, no work.
    if (!twinPath || twinPath === curPath) return;

    const encoded = twinPath.split('/').map(encodeURIComponent).join('/');

    // Raw fetch (not apiFetch) so a missing twin (404) is silent — apiFetch
    // surfaces a "Request error" toast on non-401 failures, which we don't want.
    let twin = null;
    try {
        const r = await window.fetch(
            `/api/v1/universes/${encodeURIComponent(slug)}/entries/${encoded}`,
            { credentials: 'same-origin' }
        );
        if (r.ok) twin = await r.json();
    } catch (_) { /* network blip → keep current language gracefully */ }

    if (!twin || !twin.body) return; // missing twin → keep current silently
    const md = window.CoMarkdown;
    indexEl.innerHTML = md ? md.renderMarkdown(twin.body) : esc(twin.body);
    if (md && typeof md.renderMermaidBlocks === 'function') {
        md.renderMermaidBlocks(indexEl);
    }
    body.dataset.indexPath = twinPath;
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
