// ===== CO-260 — Changelog viewer module =====
// Standalone module re-exported for use outside the main SPA.
// The full interactive viewer lives in changelog.html (inline script).
// This module exposes a minimal API for embedding the viewer in other contexts.

export { fetchChangelog, renderChangelogViewer };

/**
 * Fetch changelog data from /api/v1/changelog.
 * @param {Object} opts - { from, to, type, sort }
 * @returns {Promise<{range, versions}>}
 */
async function fetchChangelog({ from, to, type, sort } = {}) {
    const params = new URLSearchParams();
    if (from)  params.set('from', from);
    if (to)    params.set('to', to);
    if (type)  params.set('type', type);
    if (sort)  params.set('sort', sort);
    const url = '/api/v1/changelog' + (params.toString() ? '?' + params : '');
    const res = await fetch(url);
    if (!res.ok) throw new Error(`changelog API: HTTP ${res.status}`);
    return res.json();
}

/**
 * Render the changelog viewer into a container element.
 * @param {HTMLElement} container
 * @param {Object} opts - { from, to, type, sort }
 */
async function renderChangelogViewer(container, opts = {}) {
    container.innerHTML = '<div style="padding:24px;text-align:center;color:var(--text-muted,#94a3b8)">Carregando changelog…</div>';
    try {
        const data = await fetchChangelog(opts);
        const versions = data.versions || [];
        if (!versions.length) {
            container.innerHTML = '<div style="padding:24px;text-align:center;color:var(--text-muted,#94a3b8)">Nenhuma versão encontrada.</div>';
            return;
        }
        container.innerHTML = renderVersionList(versions, opts.sort);
    } catch (err) {
        container.innerHTML = `<div style="padding:24px;text-align:center;color:#e57373">Erro: ${String(err.message)}</div>`;
    }
}

function esc(s) {
    return String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function renderVersionList(versions, sort) {
    return versions.map(v => {
        const entries = sort === 'size'
            ? [...v.entries].sort((a, b) => (b.pr_size ?? 0) - (a.pr_size ?? 0))
            : v.entries;
        return `
<div style="margin-bottom:24px">
  <div style="display:flex;align-items:baseline;gap:10px;margin-bottom:8px;padding-bottom:6px;border-bottom:1px solid var(--border,#1e293b)">
    <span style="font-weight:700;font-family:monospace">v${esc(v.version)}</span>
    <span style="font-size:12px;color:var(--text-muted,#94a3b8)">${esc(v.date)}</span>
    ${v.theme ? `<span style="font-size:13px;color:var(--text-muted,#94a3b8);font-style:italic">${esc(v.theme)}</span>` : ''}
  </div>
  <div style="display:flex;flex-direction:column;gap:6px">
    ${entries.map(e => `
    <div style="display:flex;align-items:center;gap:8px;padding:8px 12px;background:var(--card,#131929);border:1px solid var(--border,#1e293b);border-radius:6px">
      <a href="${esc(e.spec_url)}" style="font-size:11px;font-weight:600;font-family:monospace;padding:2px 7px;border-radius:999px;text-decoration:none;background:#14532d;color:#86efac;border:1px solid #16a34a">${esc(e.ticket)}</a>
      <span style="flex:1;font-size:13px">${esc(e.title || e.ticket)}</span>
      ${e.pr ? `<a href="https://github.com/artelonga/co/pull/${e.pr}" target="_blank" rel="noopener" style="font-size:11px;color:var(--text-muted,#94a3b8)">#${e.pr}</a>` : ''}
    </div>`).join('')}
  </div>
</div>`;
    }).join('');
}
