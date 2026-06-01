// ===== CO-333: Feedback panel — owner review side panel + unread badge =====
// Usage:
//   import { mountFeedbackBadge, mountFeedbackPanel } from './feedback-panel.js';
//
//   mountFeedbackBadge(containerEl, universeKey, entryPath);
//     → injects a "📩 N" badge into containerEl if there's open feedback
//
//   mountFeedbackPanel(universeKey, entryPath);
//     → opens the side panel listing all feedback for that entry

import { apiFetch } from './api.js';
import { esc } from './helpers.js';

// -----------------------------------------------------------------------
// Badge — small unread indicator for the entry view
// -----------------------------------------------------------------------

export async function mountFeedbackBadge(container, universeKey, entryPath) {
    try {
        const encoded = entryPath.split('/').map(encodeURIComponent).join('/');
        const data = await apiFetch(
            `/api/v1/feedback/${encodeURIComponent(universeKey)}/entry/${encoded}`,
            {},
            true,
        );
        if (!data || !data.items) return;
        const open = data.items.filter(i => i.status === 'open').length;
        if (open === 0) return;

        const badge = document.createElement('button');
        badge.className = 'co-feedback-badge';
        badge.title = `${open} feedback(s) em aberto`;
        badge.innerHTML = `📩 <span class="co-feedback-badge-count">${open}</span>`;
        badge.style.cssText = [
            'display:inline-flex;align-items:center;gap:4px',
            'padding:2px 8px;border-radius:12px;border:1px solid var(--border,#d1d5db)',
            'background:var(--card-bg,#fff);cursor:pointer;font-size:12px',
            'color:var(--text-muted,#6b7280);margin-left:8px',
        ].join(';');
        badge.addEventListener('click', (e) => {
            e.stopPropagation();
            mountFeedbackPanel(universeKey, entryPath);
        });
        container.appendChild(badge);
    } catch (_) {}
}

// -----------------------------------------------------------------------
// Panel — full side panel with feedback list + status actions
// -----------------------------------------------------------------------

export async function mountFeedbackPanel(universeKey, entryPath) {
    // Remove any existing panel first
    document.getElementById('co-feedback-panel')?.remove();
    document.getElementById('co-feedback-panel-backdrop')?.remove();

    const backdrop = document.createElement('div');
    backdrop.id = 'co-feedback-panel-backdrop';
    backdrop.style.cssText = 'position:fixed;inset:0;background:rgba(0,0,0,.2);z-index:8000';

    const panel = document.createElement('div');
    panel.id = 'co-feedback-panel';
    panel.style.cssText = [
        'position:fixed;top:0;right:0;bottom:0;width:min(420px,96vw)',
        'background:var(--card-bg,#fff);z-index:8001;overflow-y:auto',
        'box-shadow:-4px 0 24px rgba(0,0,0,.12);padding:20px',
        'animation:co-panel-slide-in .2s ease',
    ].join(';');

    if (!document.getElementById('co-panel-styles')) {
        const s = document.createElement('style');
        s.id = 'co-panel-styles';
        s.textContent = `@keyframes co-panel-slide-in { from { transform: translateX(100%); } to { transform: translateX(0); } }`;
        document.head.appendChild(s);
    }

    const close = () => { panel.remove(); backdrop.remove(); };
    backdrop.addEventListener('click', close);

    panel.innerHTML = `
        <div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:16px">
            <h3 style="margin:0;font-size:16px">Feedback — <span style="font-weight:400;font-size:14px;opacity:.7">${esc(entryPath || 'universo')}</span></h3>
            <button id="co-fb-panel-close" style="background:none;border:none;font-size:20px;cursor:pointer;color:var(--text-muted,#888)">✕</button>
        </div>
        <div id="co-fb-panel-body" style="color:var(--text-muted,#888);font-size:14px">Carregando…</div>`;

    document.body.appendChild(backdrop);
    document.body.appendChild(panel);
    panel.querySelector('#co-fb-panel-close').addEventListener('click', close);

    await loadPanelContent(panel.querySelector('#co-fb-panel-body'), universeKey, entryPath);
}

async function loadPanelContent(container, universeKey, entryPath) {
    try {
        const encoded = entryPath
            ? entryPath.split('/').map(encodeURIComponent).join('/')
            : null;
        const url = encoded
            ? `/api/v1/feedback/${encodeURIComponent(universeKey)}/entry/${encoded}`
            : `/api/v1/feedback/${encodeURIComponent(universeKey)}`;
        const data = await apiFetch(url, {}, true);

        if (!data || !data.items || data.items.length === 0) {
            container.innerHTML = '<p>Nenhum feedback ainda.</p>';
            return;
        }

        const grouped = { open: [], reviewed: [], addressed: [] };
        for (const item of data.items) {
            (grouped[item.status] || grouped.open).push(item);
        }

        const renderGroup = (label, items) => {
            if (items.length === 0) return '';
            return `
                <h4 style="margin:16px 0 8px;font-size:13px;text-transform:uppercase;letter-spacing:.04em;opacity:.6">${esc(label)} (${items.length})</h4>
                ${items.map(renderItem).join('')}`;
        };

        container.innerHTML =
            renderGroup('Em aberto', grouped.open) +
            renderGroup('Revisado', grouped.reviewed) +
            renderGroup('Resolvido', grouped.addressed);

        container.querySelectorAll('[data-action]').forEach(btn => {
            btn.addEventListener('click', async () => {
                const id = btn.dataset.id;
                const status = btn.dataset.action;
                btn.disabled = true;
                try {
                    const resp = await fetch(`/api/v1/feedback/${encodeURIComponent(id)}`, {
                        method: 'PATCH',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ status }),
                        credentials: 'include',
                    });
                    if (resp.ok) await loadPanelContent(container, universeKey, entryPath);
                } catch (_) { btn.disabled = false; }
            });
        });
    } catch (err) {
        container.innerHTML = `<p style="color:var(--error,#ef4444)">Erro ao carregar: ${esc(err.message)}</p>`;
    }
}

function kindLabel(kind) {
    return { feedback: 'Comentário', duvida: 'Dúvida', sugestao: 'Sugestão' }[kind] || kind;
}

function renderItem(item) {
    const ts = new Date(item.created_at * 1000).toLocaleString('pt-BR');
    const from = item.name ? esc(item.name) : (item.anonymous ? 'Anônimo' : 'Visitante');
    const emailLink = item.email
        ? `<a href="mailto:${esc(item.email)}" style="color:var(--primary,#3b82f6);font-size:12px">${esc(item.email)}</a>`
        : '';

    const actions = item.status === 'open'
        ? `<button data-action="reviewed"  data-id="${esc(item.id)}" style="${actionStyle()}">Marcar revisado</button>
           <button data-action="addressed" data-id="${esc(item.id)}" style="${actionStyle()}">Marcar resolvido</button>`
        : item.status === 'reviewed'
        ? `<button data-action="addressed" data-id="${esc(item.id)}" style="${actionStyle()}">Marcar resolvido</button>
           <button data-action="open"      data-id="${esc(item.id)}" style="${actionStyle('secondary')}">Reabrir</button>`
        : `<button data-action="open" data-id="${esc(item.id)}" style="${actionStyle('secondary')}">Reabrir</button>`;

    return `
<div style="border:1px solid var(--border,#e5e7eb);border-radius:8px;padding:12px 14px;margin-bottom:10px">
  <div style="display:flex;align-items:center;gap:8px;margin-bottom:6px">
    <span style="font-size:11px;padding:2px 6px;border-radius:4px;background:var(--tag-bg,#f3f4f6)">${esc(kindLabel(item.kind))}</span>
    <span style="font-size:12px;opacity:.6">${esc(from)}</span>
    ${emailLink}
    <span style="font-size:11px;opacity:.45;margin-left:auto">${esc(ts)}</span>
  </div>
  <p style="margin:0 0 10px;font-size:14px;line-height:1.5">${esc(item.message)}</p>
  <div style="display:flex;gap:6px;flex-wrap:wrap">${actions}</div>
</div>`;
}

function actionStyle(variant = 'primary') {
    const base = 'padding:4px 10px;border-radius:5px;font-size:12px;cursor:pointer;border:1px solid';
    return variant === 'primary'
        ? `${base} var(--primary,#3b82f6);background:var(--primary,#3b82f6);color:#fff`
        : `${base} var(--border,#d1d5db);background:none;color:var(--text,#1a1a1a)`;
}
