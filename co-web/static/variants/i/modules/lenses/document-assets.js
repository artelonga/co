// CO-393: Document view — asset rendering utilities (split from conteudo.js).

import { esc } from '../../../a/modules/helpers.js';

export function shouldRenderInlinePdf(entry) {
    const fm = entry.frontmatter || {};
    return fm.type === 'reference' && fm.medium === 'pdf' && fm.file && fm.file.endsWith('.pdf');
}

export function isAssetEntry(entry) {
    const et = entry.entry_type || (entry.frontmatter || {}).type || '';
    return et.startsWith('asset.');
}

export function pdfUrlFromCard(universe, entry) {
    const fm = entry.frontmatter || {};
    const file = fm.file || '';
    return file ? `/api/v1/universes/${encodeURIComponent(universe)}/vault/${encodeURIComponent(file)}` : '';
}

export function buildPdfViewerHtml(pdfUrl, filename) {
    const viewerSrc = `/pdfjs/web/viewer.html?file=${encodeURIComponent(pdfUrl)}`;
    const dlName = filename || 'documento.pdf';
    return `<div class="pdf-viewer" id="co-pdf-viewer">
  <iframe id="co-pdf-iframe" src="${esc(viewerSrc)}" width="100%" height="800"
          loading="lazy" style="border:0;border-radius:8px;display:block" allowfullscreen></iframe>
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
    if (et === 'asset.code') {
        return `<div id="co-asset-code-container" data-asset-url="${esc(assetUrl)}" data-filename="${esc(filename)}" style="min-height:300px">
            <div class="loading-spinner" style="padding:32px"><div class="spinner"></div></div>
        </div>`;
    }
    const b = fm.size_bytes || 0;
    const sizeHuman = b < 1024 ? `${b} B` : b < 1048576 ? `${(b / 1024).toFixed(1)} KB` : `${(b / 1048576).toFixed(2)} MB`;
    return `<div style="display:flex;flex-direction:column;align-items:center;padding:48px 24px;gap:16px">
        <span class="material-symbols-outlined" style="font-size:64px;color:var(--color-muted,#888)">attach_file</span>
        <div style="font-size:1rem;font-weight:600">${esc(filename)}</div>
        <div style="font-size:.85rem;color:var(--color-muted,#888)">${esc(mime || 'application/octet-stream')} · ${esc(sizeHuman)}</div>
        ${assetUrl ? `<a href="${esc(assetUrl)}" download="${esc(filename)}" class="btn btn-secondary"><span class="material-symbols-outlined" style="font-size:16px;vertical-align:-3px">download</span> Baixar arquivo</a>` : ''}
    </div>`;
}

export function initPdfViewerActions(container) {
    const btn = container.querySelector('#co-pdf-fullscreen');
    const iframe = container.querySelector('#co-pdf-iframe');
    if (!btn || !iframe) return;
    btn.addEventListener('click', () => {
        if (iframe.requestFullscreen) iframe.requestFullscreen();
        else if (iframe.webkitRequestFullscreen) iframe.webkitRequestFullscreen();
    });
}

export async function mountAssetCodeEditor(zoomBody, opts) {
    opts = opts || {};
    const container = zoomBody.querySelector('#co-asset-code-container');
    if (!container) return;
    const assetUrl = container.dataset.assetUrl;
    const filename = container.dataset.filename || '';
    if (!assetUrl) { container.innerHTML = '<p>Arquivo sem URL</p>'; return; }
    try {
        const resp = await fetch(assetUrl);
        const text = resp.ok ? await resp.text() : `Erro ao carregar: ${resp.status}`;
        container.innerHTML = '';
        if (window.CoEditor && window.CoEditor.initCodeEditor) {
            const inst = await window.CoEditor.initCodeEditor(container, {
                content: text, filename, readOnly: !opts.editable, onSave: opts.editable ? opts.onSave : undefined,
            });
            if (inst) container._codeEditorInst = inst;
        } else if (window.CoEditor) {
            window.CoEditor.initEditor(container, { content: text, readOnly: true });
        } else {
            container.innerHTML = `<pre style="padding:16px;overflow:auto;white-space:pre-wrap;font-size:.85rem"><code>${esc(text)}</code></pre>`;
        }
    } catch (err) {
        container.innerHTML = `<p style="color:red">Erro: ${esc(String(err))}</p>`;
    }
}
