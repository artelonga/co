// ===== Conteúdo (content browser) view =====
import { state, canEditCurrentUniverse } from '../state.js';
import { api, apiFetch } from '../api.js';
import { esc, relativeDate, todayDate } from '../helpers.js';

let _openZoomModal = () => {};
let _showLoginModal = () => {};
let _showToast = () => {};
let _loadEditorBundle = async () => {};
let _showSubscribePromptModal = () => {};

export function injectConteudoCallbacks(callbacks) {
    _openZoomModal = callbacks.openZoomModal;
    _showLoginModal = callbacks.showLoginModal;
    _showToast = callbacks.showToast;
    _loadEditorBundle = callbacks.loadEditorBundle;
    if (callbacks.showSubscribePromptModal) _showSubscribePromptModal = callbacks.showSubscribePromptModal;
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

// CO-242: true when this entry is a unified asset (PDF, image, video, code, binary).
export function isAssetEntry(entry) {
    const et = entry.entry_type || (entry.frontmatter || {}).type || '';
    return et.startsWith('asset.');
}

// CO-242: build the asset viewer HTML for a given asset entry.
export function buildAssetViewerHtml(universe, entry) {
    const fm = entry.frontmatter || {};
    const et = entry.entry_type || fm.type || 'asset.binary';
    const sha256 = fm.asset_sha256 || '';
    const filename = fm.filename || entry.title || entry.path || '';
    const mime = fm.mime || '';
    const assetUrl = sha256
        ? `/api/v1/universes/${encodeURIComponent(universe)}/assets/${encodeURIComponent(sha256)}`
        : '';

    if (et === 'asset.pdf' || mime === 'application/pdf') {
        return assetUrl ? buildPdfViewerHtml(assetUrl, filename) : `<p>PDF sem URL</p>`;
    }
    if (et === 'asset.image' || mime.startsWith('image/')) {
        return assetUrl
            ? `<div style="text-align:center;padding:16px"><img src="${esc(assetUrl)}" alt="${esc(filename)}" style="max-width:100%;max-height:70vh;border-radius:8px;box-shadow:0 2px 12px rgba(0,0,0,.15)"></div>`
            : `<p>Imagem sem URL</p>`;
    }
    if (et === 'asset.video' || mime.startsWith('video/')) {
        return assetUrl
            ? `<div style="text-align:center;padding:16px"><video controls src="${esc(assetUrl)}" style="max-width:100%;max-height:70vh;border-radius:8px"></video></div>`
            : `<p>Vídeo sem URL</p>`;
    }
    // asset.code — placeholder; actual CodeMirror mount happens in mountAssetCodeEditor()
    if (et === 'asset.code') {
        return `<div id="co-asset-code-container" data-asset-url="${esc(assetUrl)}" data-filename="${esc(filename)}" style="min-height:300px">
            <div class="loading-spinner" style="padding:32px"><div class="spinner"></div></div>
        </div>`;
    }
    // asset.binary or unknown — download link
    const sizeHuman = (() => {
        const b = fm.size_bytes || 0;
        if (b < 1024) return `${b} B`;
        if (b < 1048576) return `${(b / 1024).toFixed(1)} KB`;
        return `${(b / 1048576).toFixed(2)} MB`;
    })();
    return `<div style="display:flex;flex-direction:column;align-items:center;padding:48px 24px;gap:16px">
        <span class="material-symbols-outlined" style="font-size:64px;color:var(--color-muted,#888)">attach_file</span>
        <div style="font-size:1rem;font-weight:600">${esc(filename)}</div>
        <div style="font-size:.85rem;color:var(--color-muted,#888)">${esc(mime || 'application/octet-stream')} · ${esc(sizeHuman)}</div>
        ${assetUrl ? `<a href="${esc(assetUrl)}" download="${esc(filename)}" class="btn btn-secondary">
            <span class="material-symbols-outlined" style="font-size:16px;vertical-align:-3px">download</span>
            Baixar arquivo
        </a>` : ''}
    </div>`;
}

// CO-242: if the asset code container is present, fetch the text and mount a RO CodeMirror.
async function mountAssetCodeEditor(zoomBody) {
    const container = zoomBody.querySelector('#co-asset-code-container');
    if (!container) return;
    const assetUrl = container.dataset.assetUrl;
    const filename = container.dataset.filename || '';
    if (!assetUrl) { container.innerHTML = '<p>Arquivo sem URL</p>'; return; }
    try {
        const resp = await fetch(assetUrl);
        const text = resp.ok ? await resp.text() : `Erro ao carregar: ${resp.status}`;
        container.innerHTML = '';
        if (window.CoEditor) {
            window.CoEditor.initEditor(container, { content: text, readOnly: true });
        } else {
            container.innerHTML = `<pre style="padding:16px;overflow:auto;white-space:pre-wrap;font-size:.85rem"><code>${esc(text)}</code></pre>`;
        }
    } catch (err) {
        container.innerHTML = `<p style="color:red">Erro: ${esc(String(err))}</p>`;
    }
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

/**
 * Inline detail pane controller: renders the currently selected entry
 * in the Conteúdo split layout. Master-detail with single-click-to-edit:
 *
 *   - render(): re-renders the current entry in read or edit mode
 *   - select(entry): swap to a different entry (resets to read mode)
 *   - enterEdit(): mount the CodeMirror editor in-place
 *
 * Full-screen reading routes through openZoomModal (existing zoom UI).
 * Editing is available both inline (single click on the rendered body)
 * and from the full-screen modal (its own edit button). Both edits
 * land at the same PUT endpoint, so they stay consistent.
 */
function createDetailController(container, initialEntry, deps) {
    const {
        universeSlug, allEntries, isTemplate, openZoomModal: openModal,
        showToast, entryTitle, esc, selectionKey,
    } = deps;

    let current = initialEntry;
    let editing = false;
    let editorInstance = null;
    let draftTimer = null;
    let escHandler = null;

    function draftKeyFor(entry) {
        return `co_draft_page_${encodeURIComponent(entry.path || '')}`;
    }

    function bindEscapeExit(action) {
        if (escHandler) document.removeEventListener('keydown', escHandler);
        escHandler = (e) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                action();
            }
        };
        document.addEventListener('keydown', escHandler);
    }
    function unbindEscape() {
        if (escHandler) {
            document.removeEventListener('keydown', escHandler);
            escHandler = null;
        }
    }

    function renderRead() {
        container.classList.remove('conteudo-detail-pane-editing');
        const md = window.CoMarkdown;
        let html = md ? md.renderMarkdown(current.body || '') : esc(current.body || '');
        if (md && md.resolveWikilinks) html = md.resolveWikilinks(html, universeSlug);

        container.innerHTML = `
            <div class="conteudo-detail-toolbar">
                <span class="conteudo-detail-title">
                    ${esc(entryTitle(current))}
                    <em class="conteudo-detail-hint">clique para editar</em>
                </span>
                <div class="conteudo-detail-actions">
                    <button class="conteudo-detail-btn" type="button"
                            data-detail-fullscreen title="Tela cheia (F)">
                        <span class="material-symbols-outlined" style="font-size:18px">fullscreen</span>
                    </button>
                </div>
            </div>
            <article class="conteudo-readme md-body" data-detail-body>${html}</article>
        `;

        const body = container.querySelector('[data-detail-body]');
        if (body) {
            body.querySelectorAll('table').forEach(tbl => {
                const wrap = document.createElement('div');
                wrap.className = 'co-table-wrap';
                tbl.parentNode.insertBefore(wrap, tbl);
                wrap.appendChild(tbl);
            });
            const md2 = window.CoMarkdown;
            if (md2 && md2.enableImageZoom) md2.enableImageZoom(body);
            if (md2 && md2.highlightCode) md2.highlightCode(body);
            if (md2 && md2.renderMermaidBlocks) md2.renderMermaidBlocks(body);

            // Single click on rendered content → inline edit. Skip when
            // the user clicked something interactive (link, image, etc.)
            // or when no text is selected (allows copy-on-select gestures).
            body.addEventListener('click', (e) => {
                if (e.target.closest('a, img, button, input, textarea, select')) return;
                const sel = window.getSelection && window.getSelection();
                if (sel && sel.toString().length > 0) return;
                enterEdit();
            });
        }

        const fsBtn = container.querySelector('[data-detail-fullscreen]');
        if (fsBtn) {
            fsBtn.addEventListener('click', () => {
                // Synchronous fullscreen request on the detail pane itself
                // — the browser only honors requestFullscreen when called
                // from a user-gesture handler with no intervening await,
                // so we can't route through openZoomModal (it's async).
                if (document.fullscreenElement) {
                    document.exitFullscreen?.();
                } else {
                    container.requestFullscreen?.().catch(() => {});
                }
            });
        }
    }

    function renderEdit() {
        container.classList.add('conteudo-detail-pane-editing');
        container.innerHTML = `
            <div class="conteudo-detail-toolbar conteudo-detail-toolbar-editing">
                <span class="conteudo-detail-title">
                    ${esc(entryTitle(current))}
                    <em class="conteudo-detail-mode">editando · Esc para cancelar</em>
                </span>
                <div class="conteudo-detail-actions">
                    <button class="btn btn-primary btn-sm" type="button" data-detail-save>Salvar</button>
                    <button class="btn btn-secondary btn-sm" type="button" data-detail-cancel>Cancelar</button>
                </div>
            </div>
            <textarea class="content-editor-textarea" data-detail-textarea>${esc(current.body || '')}</textarea>
        `;

        if (window.CoEditor) {
            const ta = container.querySelector('[data-detail-textarea]');
            if (ta) ta.style.display = 'none';
            const cmDiv = document.createElement('div');
            cmDiv.className = 'content-editor-cm conteudo-detail-cm';
            container.appendChild(cmDiv);
            editorInstance = window.CoEditor.initEditor(cmDiv, {
                content: current.body || '',
                // Don't sync to the hidden textarea on every keystroke —
                // it triggers DOM reflow per character. Read the value
                // from editorInstance.getValue() at save/draft time instead.
                onChange: undefined,
                readOnly: false,
            });

            if (draftTimer) clearInterval(draftTimer);
            // Save drafts every 5s. localStorage first (always cheap +
            // works offline), then fire-and-forget POST to the server
            // for the durable cross-device backup (skipped for template
            // since anon visitors can't write there).
            const draftPath = `_drafts/${current.path || ''}`;
            draftTimer = setInterval(async () => {
                if (!editorInstance) return;
                const val = editorInstance.getValue();
                try { localStorage.setItem(draftKeyFor(current), val); } catch (_) {}
                if (isTemplate) return;
                try {
                    await window.fetch(
                        `/api/v1/universes/${encodeURIComponent(universeSlug)}/entries/${draftPath.split('/').map(encodeURIComponent).join('/')}`,
                        {
                            method: 'PUT',
                            credentials: 'same-origin',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ body: val }),
                        }
                    );
                } catch (_) { /* network blip — localStorage covers it */ }
            }, 5000);
        }

        const cancelInlineEdit = () => {
            teardownEditor();
            unbindEscape();
            editing = false;
            renderRead();
        };
        bindEscapeExit(cancelInlineEdit);
        container.querySelector('[data-detail-cancel]').addEventListener('click', cancelInlineEdit);

        container.querySelector('[data-detail-save]').addEventListener('click', async () => {
            const saveBtn = container.querySelector('[data-detail-save]');
            const ta = container.querySelector('[data-detail-textarea]');
            const newBody = editorInstance && editorInstance.getContent
                ? editorInstance.getContent()
                : (ta ? ta.value : (current.body || ''));

            // 2.7.22: on the public template universe, anon visitors
            // can edit locally but can't persist server-side. The old
            // behavior showed a fake "Salvo" toast and silently
            // dropped the write — misleading, the edit was lost on
            // refresh. Now: show an honest message, open the login
            // modal, and *keep the editor open* so the user's text
            // isn't gone if they choose to sign in.
            if (isTemplate) {
                showToast(
                    window.t
                        ? window.t('save_requires_login')
                        : 'Faça login para salvar.',
                    'warning'
                );
                _showLoginModal();
                return;
            }

            if (saveBtn) { saveBtn.disabled = true; saveBtn.textContent = '...'; }
            try {
                const putRes = await window.fetch(
                    `/api/v1/universes/${encodeURIComponent(universeSlug)}/entries/${(current.path || '').split('/').map(encodeURIComponent).join('/')}`,
                    {
                        method: 'PUT',
                        credentials: 'same-origin',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ body: newBody }),
                    }
                );
                if (putRes.ok) {
                    current = { ...current, body: newBody };
                    try { localStorage.removeItem(draftKeyFor(current)); } catch (_) {}
                    // Fire-and-forget delete of the server-side draft.
                    if (!isTemplate) {
                        const draftPath = `_drafts/${current.path || ''}`;
                        window.fetch(
                            `/api/v1/universes/${encodeURIComponent(universeSlug)}/entries/${draftPath.split('/').map(encodeURIComponent).join('/')}`,
                            { method: 'DELETE', credentials: 'same-origin' }
                        ).catch(() => {});
                    }
                    showToast(window.t ? window.t('saved') : 'Salvo', 'success');
                    teardownEditor();
                    unbindEscape();
                    editing = false;
                    renderRead();
                    return;
                }

                // 2.7.23: 403 on a write means the caller isn't allowed
                // to write to this universe — but they're authed, so
                // offer to submit the body as an inline proposal that
                // lands in the target universe's _proposals/ inbox.
                if (putRes.status === 403) {
                    const accept = window.confirm(
                        'Você não tem permissão para escrever em "' + universeSlug +
                        '". Enviar essa mudança como proposta para revisão?'
                    );
                    if (accept) {
                        const propRes = await window.fetch(
                            `/api/v1/universes/${encodeURIComponent(universeSlug)}/proposals/inline`,
                            {
                                method: 'POST',
                                credentials: 'same-origin',
                                headers: { 'Content-Type': 'application/json' },
                                body: JSON.stringify({
                                    target_path: current.path || '',
                                    body: newBody,
                                }),
                            }
                        );
                        if (propRes.ok) {
                            const payload = await propRes.json().catch(() => ({}));
                            showToast(
                                'Proposta enviada (' + (payload.proposal_path || 'ok') + ')',
                                'success'
                            );
                            teardownEditor();
                            unbindEscape();
                            editing = false;
                            renderRead();
                            return;
                        }
                        showToast('Falha ao enviar proposta', 'error');
                    }
                    if (saveBtn) { saveBtn.disabled = false; saveBtn.textContent = 'Salvar'; }
                    return;
                }

                showToast('Erro ao salvar', 'error');
                if (saveBtn) { saveBtn.disabled = false; saveBtn.textContent = 'Salvar'; }
            } catch (_) {
                showToast('Erro ao salvar', 'error');
                if (saveBtn) { saveBtn.disabled = false; saveBtn.textContent = 'Salvar'; }
            }
        });
    }

    function teardownEditor() {
        if (draftTimer) { clearInterval(draftTimer); draftTimer = null; }
        if (editorInstance) {
            try { editorInstance.destroy(); } catch (_) {}
            editorInstance = null;
        }
    }

    function enterEdit() {
        if (editing) return;
        if (!isTemplate && !canEditCurrentUniverse()) {
            (deps.showSubscribePromptModal || _showSubscribePromptModal)();
            return;
        }
        editing = true;
        renderEdit();
    }

    function select(entry) {
        if (editing) teardownEditor();
        editing = false;
        current = entry;
        try { localStorage.setItem(selectionKey, entry.path || ''); } catch (_) {}
        renderRead();
    }

    renderRead();
    return { select, enterEdit, getCurrent: () => current };
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
                    <button class="co-zoom-action" id="co-zoom-fullscreen" title="Tela cheia (F)">
                        <span class="material-symbols-outlined" style="font-size:18px">fullscreen</span>
                    </button>
                </div>
            </div>
            <div class="co-zoom-body md-article" id="co-zoom-body"></div>
        </div>`;

    document.body.appendChild(overlay);

    const zoomBody = document.getElementById('co-zoom-body');

    function renderView() {
        // CO-242: asset entries (PDF, image, video, code, binary) have a dedicated
        // viewer — skip the markdown rendering path entirely.
        if (isAssetEntry(fullEntry)) {
            zoomBody.className = 'co-zoom-body co-asset-view';
            zoomBody.innerHTML = buildAssetViewerHtml(fetchUniverse, fullEntry);
            const et = fullEntry.entry_type || (fullEntry.frontmatter || {}).type || '';
            if (et === 'asset.pdf' || (fullEntry.frontmatter || {}).mime === 'application/pdf') {
                initPdfViewerActions(zoomBody);
            } else if (et === 'asset.code') {
                mountAssetCodeEditor(zoomBody);
            }
            // Asset entries are read-only — no dblclick-to-edit.
            return;
        }

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

            // 2.7.22: anon visitors get an honest prompt instead of a
            // fake-success toast (see createDetailController.renderEdit).
            if (state.isTemplate) {
                _showToast(
                    window.t
                        ? window.t('save_requires_login')
                        : 'Faça login para salvar.',
                    'warning'
                );
                _showLoginModal();
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

    const _zoomCleanupFns = [];
    function closeZoom() {
        if (_zoomDraftInterval) { clearInterval(_zoomDraftInterval); _zoomDraftInterval = null; }
        if (_zoomEditorInstance) { _zoomEditorInstance.destroy(); _zoomEditorInstance = null; }
        document.removeEventListener('keydown', onEsc);
        for (const fn of _zoomCleanupFns) { try { fn(); } catch (_) {} }
        if (document.fullscreenElement) { try { document.exitFullscreen(); } catch (_) {} }
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

    // Real fullscreen toggle: drives the browser Fullscreen API on the
    // zoom container. Esc / pressing the button again exits.
    const container = document.getElementById('co-zoom-container');
    function toggleFullscreen() {
        const fsEl = document.fullscreenElement;
        if (fsEl) {
            document.exitFullscreen?.();
        } else if (container && container.requestFullscreen) {
            container.requestFullscreen().catch(() => {});
        }
    }
    document.getElementById('co-zoom-fullscreen').addEventListener('click', toggleFullscreen);
    const onFsKey = (e) => {
        if (e.key.toLowerCase() === 'f' && !e.metaKey && !e.ctrlKey && !e.altKey) {
            const tag = (e.target && e.target.tagName) || '';
            if (tag === 'INPUT' || tag === 'TEXTAREA') return;
            // Block when inline-editing within the zoom (CodeMirror handles its own keys)
            if (zoomBody.classList.contains('co-zoom-edit-container')) return;
            e.preventDefault();
            toggleFullscreen();
        }
    };
    document.addEventListener('keydown', onFsKey);

    const onFsChange = () => {
        const inFs = !!document.fullscreenElement;
        container?.classList.toggle('co-zoom-fs', inFs);
        const icon = document.getElementById('co-zoom-fullscreen')?.querySelector('.material-symbols-outlined');
        if (icon) icon.textContent = inFs ? 'fullscreen_exit' : 'fullscreen';
    };
    document.addEventListener('fullscreenchange', onFsChange);
    _zoomCleanupFns.push(() => {
        document.removeEventListener('keydown', onFsKey);
        document.removeEventListener('fullscreenchange', onFsChange);
    });

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

    const [taskEntries, eventEntries, pageEntries, clipEntries, assetEntries, allEntries] = await Promise.all([
        api.getUniverseEntries(slug, 'task'),
        api.getUniverseEntries(slug, 'event'),
        api.getUniverseEntries(slug, 'page'),
        api.getUniverseEntries(slug, 'clip'),
        // CO-242: unified asset listing — all asset.* subtypes
        api.getUniverseEntries(slug, 'asset.*'),
        api.getUniverseEntries(slug),
    ]);

    // Hide editor scratch files from every section:
    //   `_drafts/<entry-path>`     — write-ahead-log (createDetailController.renderEdit)
    //   `_proposals/<id>.md`       — inline-proposal queue (proposal_routes.rs); these
    //                                show up in a dedicated Inbox view, not the
    //                                regular content listings.
    const isHiddenPath = (p) =>
        (p || '').startsWith('_drafts/') || (p || '').startsWith('_proposals/');
    for (const arr of [taskEntries, eventEntries, pageEntries, clipEntries, assetEntries, allEntries]) {
        for (let i = arr.length - 1; i >= 0; i--) {
            if (isHiddenPath(arr[i].path)) arr.splice(i, 1);
        }
    }

    const knownPaths = new Set([
        ...taskEntries.map(e => e.path),
        ...eventEntries.map(e => e.path),
        ...pageEntries.map(e => e.path),
        ...clipEntries.map(e => e.path),
        ...assetEntries.map(e => e.path),
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

    // README/index: render as the primary content pane. Search a small
    // priority list so universes that follow different conventions
    // (index.md, README.md) all light up. Strip it from the Páginas
    // listing so it doesn't appear twice.
    const README_PATHS = ['index.md', 'README.md', 'readme.md', 'content/index.md'];
    let readmeEntry = null;
    for (const p of README_PATHS) {
        readmeEntry = pageEntries.find(e => (e.path || '') === p)
                   || allEntries.find(e => (e.path || '') === p);
        if (readmeEntry) break;
    }
    const pagesForList = readmeEntry
        ? pageEntries.filter(e => e.path !== readmeEntry.path)
        : pageEntries;

    const pagesBodyHtml = pagesForList.length
        ? renderFolderNode(buildFolderTree(pagesForList), 0)
        : '<p class="conteudo-empty">Nenhuma página</p>';

    const tasksBodyHtml = taskEntries.length
        ? taskEntries.map(e => {
            const fm = entryFm(e);
            const taskId = fm.id || '';
            const status = fm.status || 'todo';
            const priority = fm.priority || 'medium';
            const tags = entryTags(e);
            // 2.7.26: route through the same `data-entry-path` flow as
            // page cards so the inline detail pane + click-to-edit
            // works for tasks too. The old `data-task-id` path went
            // through openContentEditor → state.tasks.find which is
            // empty on Conteúdo (state.tasks is the project-board
            // cache, never populated on the universe home view).
            return `<div class="conteudo-card conteudo-card-clickable co-page-card"
                        data-entry-path="${esc(e.path)}"
                        data-entry-title="${esc(entryTitle(e))}"
                        data-task-id="${taskId}">
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

    // CO-242: asset cards — icon + filename + mime badge, click to open viewer.
    const assetTypeIcon = (et) => {
        if (et === 'asset.pdf') return 'picture_as_pdf';
        if (et === 'asset.image') return 'image';
        if (et === 'asset.video') return 'videocam';
        if (et === 'asset.code') return 'code';
        return 'attach_file';
    };
    const assetsBodyHtml = assetEntries.length
        ? assetEntries.map(e => {
            const fm = entryFm(e);
            const et = e.entry_type || fm.type || 'asset.binary';
            const mime = fm.mime || '';
            const sizeHuman = (() => {
                const b = fm.size_bytes || 0;
                if (b < 1024) return `${b} B`;
                if (b < 1048576) return `${(b / 1024).toFixed(1)} KB`;
                return `${(b / 1048576).toFixed(2)} MB`;
            })();
            return `<div class="conteudo-card conteudo-card-clickable co-asset-card"
                        data-entry-path="${esc(e.path)}"
                        data-entry-title="${esc(entryTitle(e))}">
                <div class="conteudo-card-title" style="display:flex;align-items:center;gap:8px">
                    <span class="material-symbols-outlined" style="font-size:18px;color:var(--color-muted,#888)">${assetTypeIcon(et)}</span>
                    ${esc(entryTitle(e))}
                </div>
                <div class="conteudo-card-footer">
                    <span class="conteudo-card-date" style="font-size:.75rem">${esc(mime)}</span>
                    <span class="conteudo-card-date">${esc(sizeHuman)}</span>
                </div>
            </div>`;
          }).join('')
        : '';

    const addContentBtn = state.isTemplate
        ? ''
        : `<button class="btn btn-secondary btn-sm co-add-content" id="btn-add-content" style="margin-left:auto;font-size:12px">+ ${window.t ? window.t('add_content') : 'Adicionar conteúdo'}</button>`;

    const sectionsHtml = [
        sectionHtml('paginas', 'Páginas', pageEntries.length, pagesBodyHtml, false, null, addContentBtn),
        sectionHtml('tarefas', 'Tarefas', taskEntries.length, tasksBodyHtml, false, 'Redundante ao Kanban'),
        sectionHtml('eventos', 'Próximos Eventos', upcomingEvents.length, eventsBodyHtml, false, null),
        clipEntries.length ? sectionHtml('clipes', 'Clipes', clipEntries.length, clipsBodyHtml, false, null) : '',
        // CO-242: assets section — only shown when the universe has uploaded files.
        assetEntries.length ? sectionHtml('arquivos', 'Arquivos', assetEntries.length, assetsBodyHtml, false, null) : '',
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

    // Master-detail: left pane renders the currently *selected* entry
    // (right pane = list of cards). On desktop, clicking a card switches
    // the left pane to that entry. On mobile, clicking a card opens the
    // full-screen modal (existing behavior) — the left pane is on top
    // and stays anchored to the README/last selection.
    //
    // Selection is persisted per-universe in localStorage so refreshes
    // resume where you left off.
    const SELECTION_KEY = `co_conteudo_selected_${slug}`;
    let initialDetailEntry = readmeEntry;
    try {
        const remembered = localStorage.getItem(SELECTION_KEY);
        if (remembered) {
            const candidate = pageEntries.find(e => e.path === remembered)
                || allEntries.find(e => e.path === remembered);
            if (candidate) initialDetailEntry = candidate;
        }
    } catch (_) {}

    if (initialDetailEntry) {
        // Restore persisted layout: split percentage + Obsidian-mode flag.
        let splitPct = 50;
        try {
            const v = parseInt(localStorage.getItem('co_conteudo_split_pct') || '', 10);
            if (Number.isFinite(v) && v >= 15 && v <= 85) splitPct = v;
        } catch (_) {}
        let obsidianMode = false;
        try { obsidianMode = localStorage.getItem('co_conteudo_layout_mode') === 'obsidian'; } catch (_) {}

        content.innerHTML = `<div class="conteudo-list conteudo-with-readme">
            ${statsHtml}
            <div class="conteudo-split${obsidianMode ? ' obsidian-mode' : ''}"
                 id="conteudo-split"
                 style="--split-pct:${splitPct}%">
                <div class="conteudo-detail-pane" id="conteudo-detail-pane"></div>
                <div class="conteudo-splitter" id="conteudo-splitter"
                     role="separator" aria-orientation="vertical"
                     title="Arraste para redimensionar"></div>
                <div class="conteudo-sections-pane" id="conteudo-sections-pane">${sectionsHtml}</div>
            </div>
        </div>`;
        const detailContainer = content.querySelector('#conteudo-detail-pane');
        content._detailController = createDetailController(detailContainer, initialDetailEntry, {
            universeSlug: slug,
            allEntries: pageEntries,
            isTemplate: state.isTemplate,
            openZoomModal: _openZoomModal,
            showToast: _showToast,
            entryTitle,
            esc,
            selectionKey: SELECTION_KEY,
            showSubscribePromptModal: _showSubscribePromptModal,
        });

        // Wire the splitter drag handler.
        const splitEl = content.querySelector('#conteudo-split');
        const splitter = content.querySelector('#conteudo-splitter');
        if (splitter && splitEl) {
            let dragging = false;
            const onDown = (e) => {
                dragging = true;
                splitEl.classList.add('dragging');
                document.body.style.userSelect = 'none';
                e.preventDefault();
            };
            const onMove = (e) => {
                if (!dragging) return;
                const rect = splitEl.getBoundingClientRect();
                const isObsidian = splitEl.classList.contains('obsidian-mode');
                const pctRaw = ((e.clientX - rect.left) / rect.width) * 100;
                const pct = isObsidian ? 100 - pctRaw : pctRaw;
                const clamped = Math.max(15, Math.min(85, pct));
                splitEl.style.setProperty('--split-pct', `${clamped}%`);
                try { localStorage.setItem('co_conteudo_split_pct', String(Math.round(clamped))); } catch (_) {}
            };
            const onUp = () => {
                if (!dragging) return;
                dragging = false;
                splitEl.classList.remove('dragging');
                document.body.style.userSelect = '';
            };
            splitter.addEventListener('mousedown', onDown);
            window.addEventListener('mousemove', onMove);
            window.addEventListener('mouseup', onUp);
            // Touch
            splitter.addEventListener('touchstart', (e) => {
                if (e.touches[0]) onDown({ clientX: e.touches[0].clientX, preventDefault: () => e.preventDefault() });
            }, { passive: false });
            window.addEventListener('touchmove', (e) => {
                if (e.touches[0] && dragging) onMove({ clientX: e.touches[0].clientX });
            });
            window.addEventListener('touchend', onUp);
        }

        // Layout-mode toggle (Obsidian = narrow tree pane on the left,
        // content takes the remainder). Surfaced as a small button in
        // the sections pane toolbar so it's discoverable without
        // crowding the detail-pane toolbar.
        const sectionsPane = content.querySelector('#conteudo-sections-pane');
        if (sectionsPane) {
            const toggleBtn = document.createElement('button');
            toggleBtn.type = 'button';
            toggleBtn.className = 'conteudo-layout-toggle';
            toggleBtn.setAttribute('title', 'Modo Obsidian (árvore à esquerda)');
            toggleBtn.innerHTML = `<span class="material-symbols-outlined" style="font-size:18px">${obsidianMode ? 'dock_to_right' : 'dock_to_left'}</span>`;
            sectionsPane.appendChild(toggleBtn);
            toggleBtn.addEventListener('click', () => {
                const split = content.querySelector('#conteudo-split');
                if (!split) return;
                const wasObsidian = split.classList.toggle('obsidian-mode');
                try { localStorage.setItem('co_conteudo_layout_mode', wasObsidian ? 'obsidian' : 'default'); } catch (_) {}
                toggleBtn.innerHTML = `<span class="material-symbols-outlined" style="font-size:18px">${wasObsidian ? 'dock_to_right' : 'dock_to_left'}</span>`;
            });
        }
    } else {
        content.innerHTML = `<div class="conteudo-list">${statsHtml}${sectionsHtml}</div>`;
    }

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
                // 2.7.26: lookup spans pages + tasks (both are .co-page-card
                // after the task-card unification). Falls back to a synthetic
                // entry with body undefined so openZoomModal re-fetches.
                const entry = (content._clickableEntries || []).find(en => en.path === entryPath)
                    || { path: entryPath, title: card.dataset.entryTitle || entryPath };
                // Desktop split: click swaps the detail pane. Mobile (or
                // when no detail pane is present): open the full-screen
                // modal as before.
                const isDesktop = window.matchMedia('(min-width: 901px)').matches;
                if (isDesktop && content._detailController) {
                    content._detailController.select(entry);
                } else {
                    _openZoomModal(entry, false);
                }
            }, 220);
        });
        card.addEventListener('dblclick', () => {
            clearTimeout(clickTimer);
            const entryPath = card.dataset.entryPath;
            const entry = (content._clickableEntries || []).find(en => en.path === entryPath)
                || { path: entryPath, title: card.dataset.entryTitle || entryPath };
            _openZoomModal(entry, true);
        });
    });

    // 2.7.26: removed the data-task-id click handler. It routed through
    // openContentEditor → state.tasks.find, which is empty on the
    // Conteúdo home view (state.tasks is the project-board cache).
    // Tasks now go through the same data-entry-path flow as pages.

    content._pageEntries = pageEntries;
    content._clickableEntries = [...pageEntries, ...taskEntries];

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
