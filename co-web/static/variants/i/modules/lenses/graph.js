// CO-393: Graph lens — multi-universe relation graph (CO-345).
// Uses GET /api/v1/universes/{slug}/graph?universes=slug1,slug2
// to span child universes into the parent graph view.

import { registerLens } from './registry.js';
import { state } from '/variants/a/modules/state.js';
import { apiFetch } from '/variants/a/modules/api.js';
import { esc } from '/variants/a/modules/helpers.js';

const t = (k) => (window.t ? window.t(k) : k);

// Collect universe slugs for the graph: current + its parent (if any) + children.
function getGraphUniverses() {
    const slug = state.currentUniverseSlug;
    if (!slug) return [slug];
    const slugs = new Set([slug]);

    const info = state.universeInfo;
    if (info?.parent_key) slugs.add(info.parent_key);

    const me = state.meUniverses;
    const all = [...(me?.owned || []), ...(me?.member || []), ...(me?.subscribed || [])];
    for (const u of all) {
        if (u.parent_key === slug) slugs.add(u.key);
    }
    return Array.from(slugs);
}

async function renderGraph() {
    const content = document.querySelector('#content');
    if (!content) return;
    content.className = 'content graph-lens-view';
    content.innerHTML = `<div class="graph-lens-container" id="graph-lens-container">
        <div class="loading-spinner"><div class="spinner"></div><span>${t('loading')}</span></div>
    </div>`;

    const slug = state.currentUniverseSlug;
    if (!slug) return;

    try {
        const universes = getGraphUniverses();
        const url = `/api/v1/universes/${encodeURIComponent(slug)}/graph?universes=${universes.map(encodeURIComponent).join(',')}`;
        const data = await apiFetch(url, {}, true);
        if (!data) throw new Error('No graph data');

        const container = document.getElementById('graph-lens-container');
        if (!container) return;
        container.innerHTML = '';

        const canvas = document.createElement('canvas');
        canvas.id = 'co-graph-canvas';
        canvas.className = 'co-graph-canvas';
        canvas.style.cssText = 'width:100%;height:100%;display:block';
        container.appendChild(canvas);

        // Build node/edge arrays for co-graph renderer
        const nodes = (data.nodes || []).map(n => ({
            id: n.id,
            label: n.title || n.id,
            sublabel: n.type,
            color: typeColor(n.type),
        }));
        const edges = (data.edges || []).map(e => ({
            source: e.source,
            target: e.target,
            label: e.kind,
        }));

        if (typeof window.co_graph !== 'undefined') {
            window.co_graph.render(canvas, { nodes, edges, interactive: true });
        } else {
            renderFallbackList(container, data);
        }

        // Universe legend
        if (universes.length > 1) {
            const legend = document.createElement('div');
            legend.className = 'graph-lens-legend';
            legend.innerHTML = `<span class="graph-lens-legend-label">${t('lens.graph.universes')}:</span> ` +
                universes.map(u => `<span class="graph-lens-universe-chip">${esc(u)}</span>`).join(' ');
            container.prepend(legend);
        }
    } catch (err) {
        const container = document.getElementById('graph-lens-container');
        if (container) {
            container.innerHTML = `<div class="empty-state"><p>Erro ao carregar grafo: ${esc(String(err.message || err))}</p></div>`;
        }
    }
}

function typeColor(type) {
    const map = {
        task: '#6366f1', note: '#10b981', page: '#3b82f6',
        proposal: '#f59e0b', state: '#8b5cf6', branch: '#ec4899',
    };
    return map[type] || '#8090a0';
}

function renderFallbackList(container, data) {
    const nodes = data.nodes || [];
    const edges = data.edges || [];
    container.innerHTML = `
        <div style="padding:16px">
            <p style="color:var(--text-muted,#6b7280);font-size:13px">Grafo de relações — ${nodes.length} nós, ${edges.length} arestas</p>
            <ul style="list-style:none;padding:0;margin:0">
                ${nodes.slice(0, 50).map(n => `<li style="padding:4px 0;font-size:13px"><code>${esc(n.id)}</code> <span style="color:var(--text-muted,#6b7280)">${esc(n.type)}</span></li>`).join('')}
                ${nodes.length > 50 ? `<li style="color:var(--text-muted,#6b7280);font-size:12px">... e mais ${nodes.length - 50} nós</li>` : ''}
            </ul>
        </div>`;
}

export const graphLens = {
    id: 'graph',
    label: 'Grafo',
    icon: 'hub',
    // Graph supports all content types — useful for any universe with relations.
    supports(_contentType) { return true; },
    render() { renderGraph(); },
};

registerLens(graphLens);
