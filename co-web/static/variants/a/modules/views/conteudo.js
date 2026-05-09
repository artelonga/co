// ===== Conteúdo (content browser) view =====
import { state } from '../state.js';
import { api, apiFetch } from '../api.js';
import { esc, relativeDate, todayDate } from '../helpers.js';

let _openZoomModal = () => {};
let _showLoginModal = () => {};
let _showToast = () => {};
let _loadEditorBundle = async () => {};

export function injectConteudoCallbacks(callbacks) {
    _openZoomModal = callbacks.openZoomModal;
    _showLoginModal = callbacks.showLoginModal;
    _showToast = callbacks.showToast;
    _loadEditorBundle = callbacks.loadEditorBundle;
}

export function buildFolderTree(entries) {
    const root = { name: '', path: '', items: [], children: [] };
    for (const entry of entries) {
        let rel = entry.path || '';
        if (rel.startsWith('content/')) rel = rel.slice('content/'.length);
        const parts = rel.split('/').filter(Boolean);
        if (parts.length <= 1) {
            root.items.push(entry);
        } else {
            let node = root;
            for (let i = 0; i < parts.length - 1; i++) {
                const name = parts[i];
                let child = node.children.find(c => c.name === name);
                if (!child) {
                    child = { name, path: parts.slice(0, i + 1).join('/'), items: [], children: [] };
                    node.children.push(child);
                }
                node = child;
            }
            node.items.push(entry);
        }
    }
    return root;
}

export function countFolderEntries(node) {
    return node.items.length + node.children.reduce((s, c) => s + countFolderEntries(c), 0);
}

export function shouldRenderInlinePdf(entry) {
    const fm = entry.frontmatter || {};
    return (
        fm.type === 'reference' &&
        fm.medium === 'pdf' &&
        fm.file &&
        fm.file.endsWith('.pdf')
    );
}

export function buildPdfViewerHtml(pdfUrl, filename) {
    const viewerSrc = `/pdfjs/web/viewer.html?file=${encodeURIComponent(pdfUrl)}`;
    const dlName = filename || 'documento.pdf';
    return `<div class="pdf-viewer" id="co-pdf-viewer">
  <iframe id="co-pdf-iframe"
          src="${esc(viewerSrc)}"
          width="100%"
          height="800"
          loading="lazy"
          style="border:0;border-radius:8px;display:block"
          allowfullscreen></iframe>
  <div class="pdf-viewer-actions">
    <a href="${esc(pdfUrl)}" download="${esc(dlName)}" class="pdf-download-btn btn btn-secondary">
      <span class="material-symbols-outlined" style="font-size:16px;vertical-align:-3px">download</span>
      Baixar PDF
    </a>
    <button class="pdf-fullscreen-btn btn btn-secondary" id="co-pdf-fullscreen">
      <span class="material-symbols-outlined" style="font-size:16px;vertical-align:-3px">fullscreen</span>
      Tela cheia
    </button>
  </div>
</div>`;
}

export async function openZoomModal(entry, startInEditMode) {
    const existing = document.getElementById('co-zoom-overlay');
    if (existing) existing.remove();

    try { await _loadEditorBundle(); } catch (_) {}

    let fullEntry = entry;
    const fetchUniverse = entry._universeSlug || state.currentUniverseSlug;
    if (fullEntry.body === undefined && fullEntry.path && fetchUniverse) {
        try {
            const encodedEntryPath = (fullEntry.path || '').split('/').map(encodeURIComponent).join('/');
            const data = await apiFetch(`/api/v1/universes/${fetchUniverse}/entries/${encodedEntryPath}`);
            if (data) fullEntry = data;
        } catch (_) {}
    }

    const title = fullEntry.title || (fullEntry.frontmatter || {}).title || fullEntry.path || '';

    const overlay = document.createElement('div');
    overlay.className = 'co-zoom-overlay';
    overlay.id = 'co-zoom-overlay';
    overlay.innerHTML = `
        <div class="co-zoom-container" id="co-zoom-container">
            <div class="co-zoom-toolbar">
                <button class="co-zoom-close" id="co-zoom-close" title="Fechar (Esc)">
                    <span class="material-symbols-outlined" style="font-size:18px">close</span>
                </button>
                <span class="co-zoom-title">${esc(title)}</span>
                <div class="co-zoom-actions">
                    <button class="co-zoom-action" id="co-zoom-edit" title="Editar">
                        <span class="material-symbols-outlined" style="font-size:18px">edit</span>
                    </button>
                    <button class="co-zoom-action" id="co-zoom-dados" title="Ver dados">
                        <span class="material-symbols-outlined" style="font-size:18px">info</span>
                    </button>
                    <button class="co-zoom-action" id="co-zoom-print" title="Imprimir">
                        <span class="material-symbols-outlined" style="font-size:18px">print</span>
                    </button>
                </div>
            </div>
            <div class="co-zoom-body md-article" id="co-zoom-body"></div>
        </div>`;

    document.body.appendChild(overlay);

    const zoomBody = document.getElementById('co-zoom-body');

    function renderView() {
        const md = window.CoMarkdown;
        let html = md ? md.renderMarkdown(fullEntry.body || '') : esc(fullEntry.body || '');
        if (md && md.resolveWikilinks) html = md.resolveWikilinks(html, state.currentUniverseSlug);
        zoomBody.className = 'co-zoom-body md-article';
        zoomBody.innerHTML = html;

        zoomBody.querySelectorAll('table').forEach(tbl => {
            const wrap = document.createElement('div');
            wrap.className = 'co-table-wrap';
            tbl.parentNode.insertBefore(wrap, tbl);
            wrap.appendChild(tbl);
        });
        const md2 = window.CoMarkdown;
        if (md2 && md2.enableImageZoom) md2.enableImageZoom(zoomBody);
        if (md2 && md2.highlightCode) md2.highlightCode(zoomBody);
        if (md2 && md2.renderMermaidBlocks) md2.renderMermaidBlocks(zoomBody);

        if (shouldRenderInlinePdf(fullEntry)) {
            const pdfUrl = pdfUrlFromCard(fetchUniverse, fullEntry);
            const fm = fullEntry.frontmatter || {};
            fetch(pdfUrl, { method: 'HEAD' }).then(r => {
                if (r.ok) {
                    zoomBody.insertAdjacentHTML('beforeend', buildPdfViewerHtml(pdfUrl, fm.file || ''));
                    initPdfViewerActions(zoomBody);
                } else {
                    zoomBody.insertAdjacentHTML('beforeend',
                        `<div style="margin-top:16px;padding:12px 16px;background:#fef9c3;border-radius:8px;font-size:.85rem;color:#713f12">
                          <strong>PDF não sincronizado</strong> — execute <code>co-sync &lt;token&gt;</code>
                          para enviar os arquivos locais ao servidor.
                          ${fm.file ? `<br><span style="color:#92400e">Arquivo: ${esc(fm.file)}</span>` : ''}
                        </div>`
                    );
                }
            }).catch(() => {});
        }

        zoomBody.addEventListener('dblclick', enterEditMode, { once: true });
    }

    let _zoomEditorInstance = null;
    let _zoomDraftInterval = null;
    const draftKey = `co_draft_page_${encodeURIComponent(fullEntry.path || '')}`;

    function enterEditMode() {
        const editBtn = document.getElementById('co-zoom-edit');
        if (editBtn) editBtn.classList.add('active');

        zoomBody.className = 'co-zoom-body co-zoom-edit-container';
        zoomBody.innerHTML = `
            <textarea class="content-editor-textarea" id="co-zoom-textarea">${esc(fullEntry.body || '')}</textarea>
            <div class="co-zoom-edit-actions">
                <button class="btn btn-primary" id="co-zoom-save">Salvar</button>
                <button class="btn btn-secondary" id="co-zoom-cancel">Cancelar</button>
            </div>`;

        document.getElementById('co-zoom-cancel').addEventListener('click', () => {
            if (_zoomDraftInterval) { clearInterval(_zoomDraftInterval); _zoomDraftInterval = null; }
            if (_zoomEditorInstance) { _zoomEditorInstance.destroy(); _zoomEditorInstance = null; }
            if (editBtn) editBtn.classList.remove('active');
            renderView();
        });

        document.getElementById('co-zoom-save').addEventListener('click', async () => {
            const saveBtn = document.getElementById('co-zoom-save');
            const ta = document.getElementById('co-zoom-textarea');
            let newBody = _zoomEditorInstance && _zoomEditorInstance.getContent
                ? _zoomEditorInstance.getContent()
                : (ta ? ta.value : (fullEntry.body || ''));

            if (state.isTemplate) {
                _showToast(window.t ? window.t('saved') : 'Salvo', 'success');
                return;
            }

            if (saveBtn) { saveBtn.disabled = true; saveBtn.textContent = '...'; }
            const result = await apiFetch(
                `/api/v1/universes/${state.currentUniverseSlug}/entries/${encodeURIComponent(fullEntry.path)}`,
                { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ body: newBody }) }
            );
            if (saveBtn) { saveBtn.disabled = false; saveBtn.textContent = 'Salvar'; }
            if (result) {
                fullEntry = { ...fullEntry, body: newBody };
                try { localStorage.removeItem(draftKey); } catch (_) {}
                if (_zoomDraftInterval) { clearInterval(_zoomDraftInterval); _zoomDraftInterval = null; }
                if (_zoomEditorInstance) { _zoomEditorInstance.destroy(); _zoomEditorInstance = null; }
                if (editBtn) editBtn.classList.remove('active');
                _showToast(window.t ? window.t('saved') : 'Salvo', 'success');
                renderView();
            } else {
                _showToast('Erro ao salvar', 'error');
            }
        });

        if (window.CoEditor) {
            const ta = document.getElementById('co-zoom-textarea');
            if (ta) ta.style.display = 'none';
            const cmDiv = document.createElement('div');
            cmDiv.className = 'content-editor-cm co-zoom-cm';
            zoomBody.insertBefore(cmDiv, zoomBody.querySelector('.co-zoom-edit-actions'));
            _zoomEditorInstance = window.CoEditor.initEditor(cmDiv, {
                content: fullEntry.body || '',
                onChange: (val) => { if (ta) ta.value = val; },
                readOnly: false,
            });

            if (_zoomDraftInterval) clearInterval(_zoomDraftInterval);
            _zoomDraftInterval = setInterval(() => {
                try {
                    const val = _zoomEditorInstance ? _zoomEditorInstance.getValue() : '';
                    localStorage.setItem(draftKey, val);
                } catch (_) {}
            }, 5000);
        }
    }

    function closeZoom() {
        if (_zoomDraftInterval) { clearInterval(_zoomDraftInterval); _zoomDraftInterval = null; }
        if (_zoomEditorInstance) { _zoomEditorInstance.destroy(); _zoomEditorInstance = null; }
        document.removeEventListener('keydown', onEsc);
        const dadosOverlay = document.getElementById('co-dados-overlay');
        if (dadosOverlay) dadosOverlay.remove();
        overlay.remove();
    }

    function onEsc(e) { if (e.key === 'Escape') closeZoom(); }
    document.addEventListener('keydown', onEsc);

    document.getElementById('co-zoom-close').addEventListener('click', closeZoom);
    overlay.addEventListener('click', e => { if (e.target === overlay) closeZoom(); });

    document.getElementById('co-zoom-edit').addEventListener('click', () => {
        if (!zoomBody.classList.contains('co-zoom-edit-container')) enterEditMode();
    });

    document.getElementById('co-zoom-dados').addEventListener('click', () => {
        openViewDados(fullEntry, overlay);
    });

    document.getElementById('co-zoom-print').addEventListener('click', () => window.print());

    if (startInEditMode) { enterEditMode(); } else { renderView(); }
}

function pdfUrlFromCard(universe, entry) {
    const fm = entry.frontmatter || {};
    if (fm.blob_sha256) {
        return `/api/v1/universes/${universe}/assets/${fm.blob_sha256}`;
    }
    const dir = (entry.path || '').split('/').slice(0, -1).join('/');
    const filename = fm.file || '';
    const filePath = (dir ? dir + '/' : '') + filename;
    return `/api/v1/universes/${universe}/blob/${filePath.split('/').map(encodeURIComponent).join('/')}`;
}

function initPdfViewerActions(container) {
    const btn = container.querySelector('#co-pdf-fullscreen');
    const iframe = container.querySelector('#co-pdf-iframe');
    if (!btn || !iframe) return;
    btn.addEventListener('click', () => {
        const el = iframe;
        if (el.requestFullscreen) el.requestFullscreen();
        else if (el.webkitRequestFullscreen) el.webkitRequestFullscreen();
        else if (el.mozRequestFullScreen) el.mozRequestFullScreen();
    });
}

export function openViewDados(entry, parentEl) {
    const existingDados = document.getElementById('co-dados-overlay');
    if (existingDados) { existingDados.remove(); return; }

    const fm = entry.frontmatter || {};
    const body = entry.body || '';

    const words = body.trim() ? body.trim().split(/\s+/).length : 0;
    const chars = body.length;
    const charsNoSpaces = body.replace(/\s/g, '').length;
    const readMins = Math.max(1, Math.ceil(words / 200));
    const byteSize = new TextEncoder().encode(body).length;
    const sizeHuman = byteSize < 1024 ? `${byteSize} B`
        : byteSize < 1048576 ? `${(byteSize / 1024).toFixed(1)} KB`
        : `${(byteSize / 1048576).toFixed(2)} MB`;

    const md = window.CoMarkdown;
    let h1 = 0, h2 = 0, h3 = 0;
    if (md && md.headingCount) {
        const hc = md.headingCount(body);
        h1 = hc.h1 || 0; h2 = hc.h2 || 0; h3 = hc.h3 || 0;
    } else {
        h1 = (body.match(/^# /gm) || []).length;
        h2 = (body.match(/^## /gm) || []).length;
        h3 = (body.match(/^### /gm) || []).length;
    }
    const intLinks = (body.match(/\[\[.*?\]\]/g) || []).length;
    const extLinks = (body.match(/\[.*?\]\(https?:\/\//g) || []).length;
    const images = (body.match(/!\[.*?\]\(/g) || []).length;
    const codeBlocks = Math.floor((body.match(/^```/gm) || []).length / 2);

    const tags = Array.isArray(fm.tags) ? fm.tags : [];
    const basename = (entry.path || '').split('/').pop() || '';
    const slug = basename.replace(/\.md$/, '');
    const parentPath = (entry.path || '').split('/').slice(0, -1).join('/');
    const created = entry.created_at || fm.created || '';
    const updated = entry.updated_at || fm.modified || fm.updated || '';

    function fmtDate(iso) {
        if (!iso) return '—';
        const d = new Date(iso);
        if (isNaN(d.getTime())) return iso;
        return d.toLocaleDateString('pt-BR') + ' (' + relativeDate(iso) + ')';
    }

    const fmKeys = Object.keys(fm);
    const fmTableHtml = fmKeys.length
        ? `<table class="co-dados-fm-table"><tbody>${fmKeys.map(k => {
            const v = typeof fm[k] === 'object' ? JSON.stringify(fm[k]) : String(fm[k]);
            return `<tr><td>${esc(k)}</td><td>${esc(v)}</td></tr>`;
          }).join('')}</tbody></table>`
        : '<span style="font-size:11px;color:var(--color-text-secondary)">Sem frontmatter</span>';
    const fmRawYaml = fmKeys.map(k => {
        const v = typeof fm[k] === 'object' ? JSON.stringify(fm[k]) : fm[k];
        return `${k}: ${v}`;
    }).join('\n');

    const dadosOverlay = document.createElement('div');
    dadosOverlay.id = 'co-dados-overlay';
    dadosOverlay.className = 'co-dados-overlay';
    dadosOverlay.innerHTML = `
        <div class="co-dados-panel">
            <div class="co-dados-header">
                <span class="co-dados-title">Dados do arquivo</span>
                <button class="co-dados-close" id="co-dados-close">
                    <span class="material-symbols-outlined" style="font-size:18px">close</span>
                </button>
            </div>
            <div class="co-dados-body">
                <div class="co-dados-section">
                    <div class="co-dados-section-title">Metadados</div>
                    <div class="co-dados-row"><span class="co-dados-label">Arquivo</span><span class="co-dados-value">${esc(basename)}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Caminho</span><span class="co-dados-value" style="font-size:10px">${esc(entry.path || '')}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Tipo</span><span class="co-dados-value">${esc(entry.entry_type || fm.type || 'page')}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Slug</span><span class="co-dados-value">${esc(slug)}</span></div>
                    ${parentPath ? `<div class="co-dados-row"><span class="co-dados-label">Pasta</span><span class="co-dados-value">${esc(parentPath)}</span></div>` : ''}
                    ${tags.length ? `<div class="co-dados-row"><span class="co-dados-label">Tags</span><span class="co-dados-value">${tags.map(t => `<span class="co-dados-tag-chip">${esc(t)}</span>`).join('')}</span></div>` : ''}
                    ${fm.author ? `<div class="co-dados-row"><span class="co-dados-label">Autor</span><span class="co-dados-value">${esc(fm.author)}</span></div>` : ''}
                    <div class="co-dados-row"><span class="co-dados-label">Criado</span><span class="co-dados-value">${fmtDate(created)}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Modificado</span><span class="co-dados-value">${fmtDate(updated)}</span></div>
                </div>
                <div class="co-dados-section">
                    <div class="co-dados-section-title">Estatísticas</div>
                    <div class="co-dados-row"><span class="co-dados-label">Palavras</span><span class="co-dados-value">${words.toLocaleString('pt-BR')}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Caracteres</span><span class="co-dados-value">${chars.toLocaleString('pt-BR')}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Sem espaços</span><span class="co-dados-value">${charsNoSpaces.toLocaleString('pt-BR')}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Leitura</span><span class="co-dados-value">~${readMins} min</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Tamanho</span><span class="co-dados-value">${sizeHuman}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Títulos</span><span class="co-dados-value">H1:${h1} H2:${h2} H3:${h3}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Links</span><span class="co-dados-value">int:${intLinks} ext:${extLinks}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Imagens</span><span class="co-dados-value">${images}</span></div>
                    <div class="co-dados-row"><span class="co-dados-label">Blocos código</span><span class="co-dados-value">${codeBlocks}</span></div>
                </div>
                <div class="co-dados-section">
                    <div class="co-dados-section-title">
                        Frontmatter
                        <button id="co-dados-fm-toggle" style="font-size:10px;background:none;border:none;cursor:pointer;color:var(--accent);margin-left:8px">Ver YAML bruto</button>
                    </div>
                    <div id="co-dados-fm-table">${fmTableHtml}</div>
                    <pre class="co-dados-fm-raw" id="co-dados-fm-raw">${esc(fmRawYaml || '(vazio)')}</pre>
                </div>
                <div class="co-dados-section">
                    <div class="co-dados-section-title">Ações</div>
                    <div class="co-dados-actions">
                        <button class="co-dados-action-btn" id="co-dados-copy-path">
                            <span class="material-symbols-outlined" style="font-size:14px">content_copy</span>Copiar caminho
                        </button>
                        <button class="co-dados-action-btn" id="co-dados-copy-fm">
                            <span class="material-symbols-outlined" style="font-size:14px">data_object</span>Copiar frontmatter como JSON
                        </button>
                        <button class="co-dados-action-btn" id="co-dados-download">
                            <span class="material-symbols-outlined" style="font-size:14px">download</span>Baixar como .md
                        </button>
                    </div>
                </div>
            </div>
        </div>`;

    (parentEl || document.body).appendChild(dadosOverlay);

    document.getElementById('co-dados-close').addEventListener('click', () => dadosOverlay.remove());

    document.getElementById('co-dados-fm-toggle').addEventListener('click', () => {
        const raw = document.getElementById('co-dados-fm-raw');
        const table = document.getElementById('co-dados-fm-table');
        const btn = document.getElementById('co-dados-fm-toggle');
        const showing = raw.classList.contains('visible');
        raw.classList.toggle('visible', !showing);
        table.style.display = showing ? '' : 'none';
        btn.textContent = showing ? 'Ver YAML bruto' : 'Ver tabela';
    });

    document.getElementById('co-dados-copy-path').addEventListener('click', () => {
        navigator.clipboard.writeText(entry.path || '').then(() => _showToast('Caminho copiado', 'success'));
    });

    document.getElementById('co-dados-copy-fm').addEventListener('click', () => {
        navigator.clipboard.writeText(JSON.stringify(fm, null, 2)).then(() => _showToast('Frontmatter copiado', 'success'));
    });

    document.getElementById('co-dados-download').addEventListener('click', () => {
        const blob = new Blob([body], { type: 'text/markdown;charset=utf-8' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = basename || 'arquivo.md';
        document.body.appendChild(a);
        a.click();
        setTimeout(() => { document.body.removeChild(a); URL.revokeObjectURL(url); }, 100);
    });
}

export async function renderConteudo() {
    const content = document.querySelector('#content');
    content.className = 'content conteudo-view';
    content.innerHTML = '<div class="loading-spinner"><div class="spinner"></div><p>Carregando...</p></div>';

    const slug = state.currentUniverseSlug;

    try { await _loadEditorBundle(); } catch (_) {}

    const [taskEntries, eventEntries, pageEntries, clipEntries, allEntries] = await Promise.all([
        api.getUniverseEntries(slug, 'task'),
        api.getUniverseEntries(slug, 'event'),
        api.getUniverseEntries(slug, 'page'),
        api.getUniverseEntries(slug, 'clip'),
        api.getUniverseEntries(slug),
    ]);

    const knownPaths = new Set([
        ...taskEntries.map(e => e.path),
        ...eventEntries.map(e => e.path),
        ...pageEntries.map(e => e.path),
        ...clipEntries.map(e => e.path),
    ]);
    const CONTEUDO_BACKEND_TYPES = new Set(['state', 'branch', 'proposal', 'merge']);
    for (const e of allEntries) {
        const t = (e.frontmatter || {}).type;
        if (CONTEUDO_BACKEND_TYPES.has(t)) continue;
        if (!knownPaths.has(e.path) && (e.path || '').endsWith('.md')) {
            pageEntries.push(e);
        }
    }

    function entryFm(e) { return e.frontmatter || {}; }
    function entryTitle(e) { return e.title || entryFm(e).title || e.path || ''; }
    function entryTags(e) {
        const tags = entryFm(e).tags;
        return Array.isArray(tags) ? tags : [];
    }

    function cardBodyHtml(body, fm) {
        if (body && body.trim().length > 0) {
            const md = window.CoMarkdown;
            if (md) {
                return `<div class="conteudo-card-body md-body md-fade">${md.renderMarkdown(body)}</div>`;
            }
            const snippet = body.slice(0, 200);
            return `<div class="conteudo-card-body">${esc(snippet)}${body.length > 200 ? '…' : ''}</div>`;
        }
        return frontmatterPreviewHtml(fm);
    }

    const FM_HIDE_KEYS = new Set([
        'type', 'slug', 'order', 'tags', 'created', 'modified',
        'created_at', 'updated_at', 'next_id', 'archived', 'archived_at',
        'frontmatter', 'parent', 'id', 'key', 'project_key',
    ]);

    function frontmatterPreviewHtml(fm) {
        if (!fm || typeof fm !== 'object') return '';
        const entries = Object.entries(fm)
            .filter(([k, v]) => !FM_HIDE_KEYS.has(k))
            .filter(([, v]) => {
                if (v === null || v === undefined || v === '') return false;
                if (Array.isArray(v) && v.length === 0) return false;
                return true;
            });
        if (!entries.length) return '';
        const fmtVal = (v) => {
            if (Array.isArray(v)) return v.map(x => esc(String(x))).join(', ');
            if (typeof v === 'object') {
                try { return esc(JSON.stringify(v)); } catch (_) { return ''; }
            }
            const s = String(v);
            if (/^https?:\/\/|^\//.test(s) && /\.(png|jpe?g|gif|svg|webp)(\?|$)/i.test(s)) {
                return `<img src="${esc(s)}" alt="" loading="lazy" class="conteudo-fm-img">`;
            }
            if (/^https?:\/\//.test(s)) {
                return `<a href="${esc(s)}" target="_blank" rel="noopener">${esc(s)}</a>`;
            }
            return esc(s);
        };
        const rows = entries.slice(0, 8).map(([k, v]) =>
            `<div class="conteudo-fm-row"><span class="conteudo-fm-key">${esc(k)}</span><span class="conteudo-fm-val">${fmtVal(v)}</span></div>`
        ).join('');
        return `<div class="conteudo-card-body conteudo-fm-preview">${rows}</div>`;
    }

    function sectionOpen(key, defaultOpen) {
        try {
            const v = localStorage.getItem(`co_section_${key}`);
            return v === null ? defaultOpen : v === '1';
        } catch (_) { return defaultOpen; }
    }

    function sectionHtml(key, label, count, bodyHtml, defaultOpen, tooltip, actionBtn) {
        const open = sectionOpen(key, defaultOpen);
        const tooltipAttr = tooltip ? ` title="${esc(tooltip)}"` : '';
        const actionHtml = actionBtn || '';
        return `<div class="co-section" data-section="${esc(key)}">
            <div class="co-section-header" data-section-toggle${tooltipAttr}>
                <span class="co-section-chevron ${open ? 'open' : 'closed'}">▼</span>
                <span class="co-section-title">${esc(label)}</span>
                <span class="co-section-count">${count}</span>
                ${actionHtml}
            </div>
            <div class="co-section-body${open ? '' : ' collapsed'}">${bodyHtml}</div>
        </div>`;
    }

    function renderPageCard(e) {
        const tags = entryTags(e);
        const fm = entryFm(e);
        const updated = e.updated_at || fm.modified || fm.updated || '';
        const relDate = relativeDate(updated);
        return `<div class="conteudo-card conteudo-card-clickable co-page-card"
                    data-entry-path="${esc(e.path)}"
                    data-entry-title="${esc(entryTitle(e))}">
            <div class="conteudo-card-title">${esc(entryTitle(e))}</div>
            ${cardBodyHtml(e.body, entryFm(e))}
            <div class="conteudo-card-footer">
                ${tags.length ? `<div class="conteudo-card-tags">${tags.map(t => `<span class="conteudo-tag">#${esc(t)}</span>`).join('')}</div>` : '<span></span>'}
                ${relDate ? `<span class="conteudo-card-date">${esc(relDate)}</span>` : ''}
            </div>
        </div>`;
    }

    function renderFolderNode(node, depth) {
        let html = '';
        if (depth === 0) {
            html += node.items.map(renderPageCard).join('');
        }
        for (const child of node.children) {
            const folderKey = `co_folder_${encodeURIComponent(child.path)}`;
            let savedState = null;
            try { savedState = localStorage.getItem(folderKey); } catch (_) {}
            const isOpen = savedState === 'open';
            const count = countFolderEntries(child);
            html += `<div class="co-folder" data-folder-path="${esc(child.path)}" data-folder-key="${esc(folderKey)}">
                <div class="co-folder-header" data-folder-toggle>
                    <span class="co-folder-chevron">${isOpen ? '▼' : '▶'}</span>
                    <span class="material-symbols-outlined co-folder-icon">${isOpen ? 'folder_open' : 'folder'}</span>
                    <span class="co-folder-name">${esc(child.name)}</span>
                    <span class="co-folder-count">${count}</span>
                </div>
                <div class="co-folder-body"${isOpen ? '' : ' style="display:none"'}>
                    ${child.items.map(renderPageCard).join('')}
                    ${renderFolderNode(child, depth + 1)}
                </div>
            </div>`;
        }
        return html;
    }

    const pagesBodyHtml = pageEntries.length
        ? renderFolderNode(buildFolderTree(pageEntries), 0)
        : '<p class="conteudo-empty">Nenhuma página</p>';

    const tasksBodyHtml = taskEntries.length
        ? taskEntries.map(e => {
            const fm = entryFm(e);
            const taskId = fm.id || '';
            const status = fm.status || 'todo';
            const priority = fm.priority || 'medium';
            const tags = entryTags(e);
            return `<div class="conteudo-card conteudo-card-clickable" data-task-id="${taskId}">
                <div class="conteudo-card-meta">${esc(status)} · ${esc(priority)}</div>
                <div class="conteudo-card-title">${esc(entryTitle(e))}</div>
                ${cardBodyHtml(e.body, entryFm(e))}
                ${tags.length ? `<div class="conteudo-card-tags">${tags.map(t => `<span class="conteudo-tag">${esc(t)}</span>`).join('')}</div>` : ''}
            </div>`;
          }).join('')
        : '<p class="conteudo-empty">Nenhuma tarefa</p>';

    const today = todayDate();
    const upcomingEvents = eventEntries
        .filter(e => { const d = entryFm(e).date || entryFm(e).data || ''; return d >= today; })
        .sort((a, b) => (entryFm(a).date || entryFm(a).data || '').localeCompare(entryFm(b).date || entryFm(b).data || ''))
        .slice(0, 5);

    const eventsBodyHtml = upcomingEvents.length
        ? upcomingEvents.map(e => {
            const fm = entryFm(e);
            const date = fm.date || fm.data || '';
            const local = fm.local || fm.location || '';
            return `<div class="conteudo-card">
                <div class="conteudo-card-meta">${esc(date)}${local ? ' · ' + esc(local) : ''}</div>
                <div class="conteudo-card-title">${esc(entryTitle(e))}</div>
                ${cardBodyHtml(e.body, entryFm(e))}
            </div>`;
          }).join('')
        : '<p class="conteudo-empty">Nenhum evento próximo</p>';

    const clipsBodyHtml = clipEntries.length
        ? clipEntries.slice(0, 6).map(e => {
            const fm = entryFm(e);
            const url = fm.url || fm.link || '';
            return `<div class="conteudo-card">
                <div class="conteudo-card-title">${url ? `<a href="${esc(url)}" target="_blank" rel="noopener">${esc(entryTitle(e))}</a>` : esc(entryTitle(e))}</div>
                ${cardBodyHtml(e.body, entryFm(e))}
            </div>`;
          }).join('')
        : '<p class="conteudo-empty">Nenhum clipe</p>';

    const addContentBtn = state.isTemplate
        ? ''
        : `<button class="btn btn-secondary btn-sm co-add-content" id="btn-add-content" style="margin-left:auto;font-size:12px">+ ${window.t ? window.t('add_content') : 'Adicionar conteúdo'}</button>`;

    const sectionsHtml = [
        sectionHtml('paginas', 'Páginas', pageEntries.length, pagesBodyHtml, false, null, addContentBtn),
        sectionHtml('tarefas', 'Tarefas', taskEntries.length, tasksBodyHtml, false, 'Redundante ao Kanban'),
        sectionHtml('eventos', 'Próximos Eventos', upcomingEvents.length, eventsBodyHtml, false, null),
        clipEntries.length ? sectionHtml('clipes', 'Clipes', clipEntries.length, clipsBodyHtml, false, null) : '',
    ].join('');

    const totalEntries = allEntries.length;
    const lastUpdated = allEntries
        .map(e => e.updated_at || (e.frontmatter || {}).modified || '')
        .filter(Boolean)
        .sort()
        .slice(-1)[0] || '';
    const lastUpdatedRel = lastUpdated ? relativeDate(lastUpdated) : '';
    const tagCount = (() => {
        const seen = new Set();
        for (const e of allEntries) {
            const t = (e.frontmatter || {}).tags;
            if (Array.isArray(t)) t.forEach(x => seen.add(x));
        }
        return seen.size;
    })();
    const statsHtml = `
        <div class="conteudo-stats">
            <div class="conteudo-stat"><span class="conteudo-stat-value">${totalEntries}</span><span class="conteudo-stat-label">${totalEntries === 1 ? 'entrada' : 'entradas'}</span></div>
            <div class="conteudo-stat"><span class="conteudo-stat-value">${pageEntries.length}</span><span class="conteudo-stat-label">${pageEntries.length === 1 ? 'página' : 'páginas'}</span></div>
            <div class="conteudo-stat"><span class="conteudo-stat-value">${taskEntries.length}</span><span class="conteudo-stat-label">${taskEntries.length === 1 ? 'tarefa' : 'tarefas'}</span></div>
            <div class="conteudo-stat"><span class="conteudo-stat-value">${eventEntries.length}</span><span class="conteudo-stat-label">${eventEntries.length === 1 ? 'evento' : 'eventos'}</span></div>
            <div class="conteudo-stat"><span class="conteudo-stat-value">${tagCount}</span><span class="conteudo-stat-label">${tagCount === 1 ? 'tag' : 'tags'}</span></div>
            ${lastUpdatedRel ? `<div class="conteudo-stat conteudo-stat-meta"><span class="conteudo-stat-label">Última edição</span><span class="conteudo-stat-value-meta">${esc(lastUpdatedRel)}</span></div>` : ''}
        </div>`;

    content.innerHTML = `<div class="conteudo-list">${statsHtml}${sectionsHtml}</div>`;

    content.querySelectorAll('[data-section-toggle]').forEach(header => {
        header.addEventListener('click', () => {
            const section = header.closest('[data-section]');
            const key = section.dataset.section;
            const body = section.querySelector('.co-section-body');
            const chevron = header.querySelector('.co-section-chevron');
            const isOpen = !body.classList.contains('collapsed');
            body.classList.toggle('collapsed', isOpen);
            chevron.classList.toggle('open', !isOpen);
            chevron.classList.toggle('closed', isOpen);
            try { localStorage.setItem(`co_section_${key}`, isOpen ? '0' : '1'); } catch (_) {}
        });
    });

    content.querySelectorAll('[data-folder-toggle]').forEach(header => {
        header.addEventListener('click', () => {
            const folder = header.closest('.co-folder');
            const folderKey = folder.dataset.folderKey;
            const body = folder.querySelector('.co-folder-body');
            const chevron = header.querySelector('.co-folder-chevron');
            const icon = header.querySelector('.co-folder-icon');
            const isOpen = body.style.display !== 'none';
            body.style.display = isOpen ? 'none' : '';
            chevron.textContent = isOpen ? '▶' : '▼';
            if (icon) icon.textContent = isOpen ? 'folder' : 'folder_open';
            try { localStorage.setItem(folderKey, isOpen ? 'closed' : 'open'); } catch (_) {}
        });
    });

    content.querySelectorAll('.co-page-card').forEach(card => {
        let clickTimer = null;
        card.addEventListener('click', e => {
            if (e.detail >= 2) return;
            clearTimeout(clickTimer);
            clickTimer = setTimeout(() => {
                const entryPath = card.dataset.entryPath;
                const entry = (content._pageEntries || []).find(en => en.path === entryPath)
                    || { path: entryPath, title: card.dataset.entryTitle || entryPath, body: '' };
                _openZoomModal(entry, false);
            }, 220);
        });
        card.addEventListener('dblclick', () => {
            clearTimeout(clickTimer);
            const entryPath = card.dataset.entryPath;
            const entry = (content._pageEntries || []).find(en => en.path === entryPath)
                || { path: entryPath, title: card.dataset.entryTitle || entryPath, body: '' };
            _openZoomModal(entry, true);
        });
    });

    content.querySelectorAll('[data-task-id]').forEach(card => {
        card.addEventListener('click', () => {
            const taskId = parseInt(card.dataset.taskId);
            if (taskId) _openContentEditor(taskId);
        });
    });

    content._pageEntries = pageEntries;

    const addBtn = document.getElementById('btn-add-content');
    if (addBtn) {
        addBtn.addEventListener('click', async (e) => {
            e.stopPropagation();
            if (state.isTemplate) { _showLoginModal(); return; }

            const title = prompt(window.t ? window.t('new_page_title') : 'Título da nova página:');
            if (!title || !title.trim()) return;

            const slugVal = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 60);
            const path = `content/${slugVal}.md`;
            const slug2 = state.currentUniverseSlug;

            const result = await apiFetch(`/api/v1/universes/${slug2}/entries`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    path: path,
                    type: 'page',
                    frontmatter: { type: 'page', title: title.trim(), slug: slugVal, tags: [], created: new Date().toISOString(), modified: new Date().toISOString() },
                    body: `# ${title.trim()}\n\n`,
                }),
            }, true);

            if (result) {
                _openZoomModal({ path, title: title.trim(), body: `# ${title.trim()}\n\n` }, true);
            } else {
                _showToast('Erro ao criar página', 'error');
            }
        });
    }
}

// Callback for content editor (injected from app.js)
let _openContentEditor = () => {};
export function injectOpenContentEditor(fn) { _openContentEditor = fn; }
