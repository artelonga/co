// ===== Changelog view — CO-129 =====
// Jujutsu-shaped op-log DAG renderer.
// Reads /api/v1/universes/{slug}/oplog and produces a virtualized SVG timeline.
import { state } from '../state.js';
import { api } from '../api.js';
import { esc } from '../helpers.js';

let _showToast = () => {};
let _showLoginModal = () => {};

export function injectChangelogCallbacks(callbacks) {
    if (callbacks.showToast) _showToast = callbacks.showToast;
    if (callbacks.showLoginModal) _showLoginModal = callbacks.showLoginModal;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const NODE_HEIGHT = 84;   // px per node slot
const DOT_X = 28;         // x-center of the node dot
const LINE_X = DOT_X;     // x of the connecting line

// Node-type glyphs and colors (resolved via CSS vars at render time).
const NODE_GLYPH = {
    commit:   '●',
    promote:  '⬡',
    conflict: '⊗',
    revert:   '⟳',
};

// ---------------------------------------------------------------------------
// State local to this view
// ---------------------------------------------------------------------------

let _nodes = [];
let _nextAfterSeq = null;
let _loading = false;
let _selectedId = null;

// ---------------------------------------------------------------------------
// Public render entry point
// ---------------------------------------------------------------------------

export async function renderChangelog() {
    const content = document.querySelector('#content');
    content.className = 'content changelog-mode';

    const slug = state.currentUniverseSlug;
    if (!slug) {
        content.innerHTML = '<p class="empty-state">Selecione um universo</p>';
        return;
    }

    // Reset local state on each full render.
    _nodes = [];
    _nextAfterSeq = null;
    _selectedId = null;

    content.innerHTML = `
        <div class="changelog-root">
            <div class="changelog-toolbar">
                <span class="changelog-title" id="changelog-title">Histórico</span>
                <span class="changelog-count" id="changelog-count"></span>
                <button class="btn btn-secondary btn-sm" id="changelog-load-more" style="display:none">
                    Carregar mais
                </button>
            </div>
            <div class="changelog-body">
                <div class="changelog-dag-wrap" id="changelog-dag-wrap">
                    <div class="changelog-empty" id="changelog-empty">
                        <span class="material-symbols-outlined" style="font-size:32px;opacity:.4">history</span>
                        <p>Carregando histórico…</p>
                    </div>
                    <svg class="changelog-svg" id="changelog-svg"
                         xmlns="http://www.w3.org/2000/svg"
                         style="display:none">
                    </svg>
                    <div class="changelog-nodes" id="changelog-nodes" style="display:none"></div>
                </div>
                <aside class="changelog-detail hidden" id="changelog-detail">
                    <div class="changelog-detail-inner" id="changelog-detail-inner"></div>
                </aside>
            </div>
        </div>`;

    document.getElementById('changelog-load-more').addEventListener('click', loadMore);

    await loadPage(slug, null, 200);
}

// ---------------------------------------------------------------------------
// Data loading
// ---------------------------------------------------------------------------

async function loadPage(slug, afterSeq, limit) {
    if (_loading) return;
    _loading = true;
    try {
        const data = await api.getOplog(slug, afterSeq, limit);
        if (!data) {
            showEmpty('Não foi possível carregar o histórico. Verifique se está autenticado.');
            return;
        }
        _nodes = _nodes.concat(data.nodes || []);
        _nextAfterSeq = data.next_after_seq ?? null;
        renderDag();
    } finally {
        _loading = false;
    }
}

async function loadMore() {
    const slug = state.currentUniverseSlug;
    if (!slug || _nextAfterSeq == null) return;
    await loadPage(slug, _nextAfterSeq, 200);
}

function showEmpty(msg) {
    const el = document.getElementById('changelog-empty');
    if (el) {
        el.innerHTML = `<span class="material-symbols-outlined" style="font-size:32px;opacity:.4">history</span><p>${esc(msg)}</p>`;
        el.style.display = 'flex';
    }
}

// ---------------------------------------------------------------------------
// DAG rendering (virtualized via content-visibility CSS)
// ---------------------------------------------------------------------------

function renderDag() {
    const nodes = _nodes;

    const emptyEl = document.getElementById('changelog-empty');
    const svgEl = document.getElementById('changelog-svg');
    const nodesEl = document.getElementById('changelog-nodes');
    const countEl = document.getElementById('changelog-count');
    const loadMoreBtn = document.getElementById('changelog-load-more');

    if (!svgEl || !nodesEl) return;

    if (nodes.length === 0) {
        showEmpty('Este universo ainda não tem histórico de operações.');
        return;
    }

    emptyEl.style.display = 'none';

    // Total height of the DAG area.
    const totalH = nodes.length * NODE_HEIGHT + 32;
    const svgW = 540;

    // Build SVG: vertical spine + node dots.
    // The spine runs the full height; each dot sits at its node's vertical midpoint.
    let svgParts = [
        `<line x1="${LINE_X}" y1="0" x2="${LINE_X}" y2="${totalH}"`,
        ` class="dag-spine" stroke="var(--border,#e5e7eb)" stroke-width="2"/>`,
    ];

    for (let i = 0; i < nodes.length; i++) {
        const n = nodes[i];
        const cy = i * NODE_HEIGHT + NODE_HEIGHT / 2;
        const cls = `dag-dot dag-dot-${n.node_type}`;
        svgParts.push(
            `<circle cx="${DOT_X}" cy="${cy}" r="7" class="${cls}" data-id="${esc(n.id)}"/>`,
        );
        // Branch label if present (future: from branching API)
        if (n.branch) {
            svgParts.push(
                `<rect x="${DOT_X + 12}" y="${cy - 9}" width="${n.branch.length * 7 + 8}" height="18"`,
                ` rx="9" class="dag-branch-bg"/>`,
                `<text x="${DOT_X + 16}" y="${cy + 4}" class="dag-branch-label">${esc(n.branch)}</text>`,
            );
        }
    }

    svgEl.setAttribute('width', svgW);
    svgEl.setAttribute('height', totalH);
    svgEl.innerHTML = svgParts.join('');
    svgEl.style.display = 'block';

    // Build HTML nodes (overlaid on SVG via absolute positioning).
    const rows = nodes.map((n, i) => {
        const top = i * NODE_HEIGHT;
        const ts = formatTs(n.ts_micros);
        const author = esc(n.author_id || 'anon');
        const idShort = esc(shortId(n.id));
        const summary = esc(n.summary || '');
        const typeLabel = n.node_type !== 'commit' ? `<span class="dag-node-type-badge badge-${n.node_type}">${esc(n.node_type)}</span>` : '';
        const selected = n.id === _selectedId ? ' dag-node--selected' : '';
        return `<div class="dag-node${selected}" data-id="${esc(n.id)}" data-seq="${n.seq}"
                     style="top:${top}px;height:${NODE_HEIGHT}px">
            <div class="dag-node-content">
                <div class="dag-node-header">
                    <span class="dag-node-id">${idShort}</span>
                    ${typeLabel}
                    <span class="dag-node-time">${ts}</span>
                    <span class="dag-node-author">${author}</span>
                </div>
                <div class="dag-node-summary">${summary}</div>
            </div>
        </div>`;
    }).join('');

    nodesEl.style.cssText = `display:block;position:absolute;top:0;left:${DOT_X + 14}px;right:0`;
    nodesEl.innerHTML = rows;
    nodesEl.style.height = totalH + 'px';

    // Make the SVG container position relative so nodes overlay it.
    const wrap = document.getElementById('changelog-dag-wrap');
    if (wrap) {
        wrap.style.cssText = 'position:relative;overflow-y:auto;flex:1;min-height:0';
    }

    // Update count + load-more
    if (countEl) {
        countEl.textContent = `${nodes.length} operaç${nodes.length === 1 ? 'ão' : 'ões'}`;
    }
    if (loadMoreBtn) {
        loadMoreBtn.style.display = _nextAfterSeq != null ? 'inline-flex' : 'none';
    }

    // Bind node click events.
    nodesEl.addEventListener('click', onNodeClick);

    // Also bind SVG dot clicks.
    svgEl.addEventListener('click', (e) => {
        const dot = e.target.closest('.dag-dot');
        if (dot) onNodeClickById(dot.dataset.id);
    });
}

// ---------------------------------------------------------------------------
// Node click → detail panel
// ---------------------------------------------------------------------------

function onNodeClick(e) {
    const nodeEl = e.target.closest('.dag-node');
    if (!nodeEl) return;
    onNodeClickById(nodeEl.dataset.id);
}

function onNodeClickById(id) {
    _selectedId = id;

    // Update selected highlight.
    document.querySelectorAll('.dag-node').forEach(el => {
        el.classList.toggle('dag-node--selected', el.dataset.id === id);
    });
    document.querySelectorAll('.dag-dot').forEach(el => {
        el.classList.toggle('dag-dot--selected', el.dataset.id === id);
    });

    const node = _nodes.find(n => n.id === id);
    if (!node) return;

    const panel = document.getElementById('changelog-detail');
    const inner = document.getElementById('changelog-detail-inner');
    if (!panel || !inner) return;

    panel.classList.remove('hidden');
    inner.innerHTML = buildDetailPanel(node);

    const restoreBtn = document.getElementById('changelog-restore-btn');
    if (restoreBtn) {
        restoreBtn.addEventListener('click', () => onRestore(node));
    }
}

function buildDetailPanel(node) {
    const ts = formatTs(node.ts_micros);
    const idShort = shortId(node.id);
    const changes = node.changes || [];
    const typeLabel = node.node_type !== 'commit'
        ? `<span class="dag-node-type-badge badge-${esc(node.node_type)}">${esc(node.node_type)}</span>`
        : '';

    const changeRows = changes.map(c => {
        const icon = c.op === 'added' ? '+' : c.op === 'deleted' ? '−' : '~';
        const cls = `diff-${c.op}`;
        const fname = c.path.split('/').pop();
        return `<div class="diff-row ${cls}">
            <span class="diff-icon">${icon}</span>
            <span class="diff-path" title="${esc(c.path)}">${esc(fname)}</span>
            <span class="diff-full-path">${esc(c.path)}</span>
        </div>`;
    }).join('');

    const canRestore = state.me && (
        state.universeInfo?.owner_id === state.me.user_id ||
        state.me.tier === 'admin'
    );

    return `
        <div class="detail-header">
            <h3 class="detail-title">
                <span class="detail-id">${esc(idShort)}</span>
                ${typeLabel}
            </h3>
            <button class="btn-icon detail-close" id="changelog-detail-close" title="Fechar">×</button>
        </div>
        <div class="detail-meta">
            <span class="material-symbols-outlined" style="font-size:14px">schedule</span>
            ${esc(ts)}
            ${node.author_id ? `<span>·</span><span class="material-symbols-outlined" style="font-size:14px">person</span>${esc(node.author_id)}` : ''}
        </div>
        <p class="detail-summary">${esc(node.summary || '')}</p>
        <div class="detail-changes">
            <h4 class="detail-changes-title">Mudanças (${changes.length})</h4>
            <div class="diff-list">${changeRows || '<p class="form-hint">Sem mudanças registradas.</p>'}</div>
        </div>
        ${canRestore ? `
        <div class="detail-actions">
            <button class="btn btn-warning" id="changelog-restore-btn">
                <span class="material-symbols-outlined" style="font-size:16px;vertical-align:middle">history</span>
                Restaurar até aqui
            </button>
            <p class="form-hint" style="margin-top:6px">
                ⚠ Esta ação reescreve o estado atual do universo. Não pode ser desfeita.
            </p>
        </div>` : ''}`;
}

async function onRestore(node) {
    const slug = state.currentUniverseSlug;
    if (!slug) return;
    const ok = window.confirm(
        `Restaurar o universo ao estado da operação ${shortId(node.id)}?\n\nEsta ação reescreve o estado atual e não pode ser desfeita.`,
    );
    if (!ok) return;

    const btn = document.getElementById('changelog-restore-btn');
    if (btn) { btn.disabled = true; btn.textContent = 'Restaurando…'; }

    const result = await api.revertToOp(slug, node.seq);
    if (result) {
        _showToast(`Restaurado: ${result.restored} arquivos restaurados, ${result.deleted} removidos.`, 'success');
        // Reload changelog after restore.
        renderChangelog();
    } else {
        if (btn) { btn.disabled = false; btn.textContent = 'Restaurar até aqui'; }
    }
}

// Close detail panel handler (delegated from the panel's close button).
document.addEventListener('click', (e) => {
    if (e.target && e.target.id === 'changelog-detail-close') {
        const panel = document.getElementById('changelog-detail');
        if (panel) panel.classList.add('hidden');
        _selectedId = null;
        document.querySelectorAll('.dag-node').forEach(el => el.classList.remove('dag-node--selected'));
        document.querySelectorAll('.dag-dot').forEach(el => el.classList.remove('dag-dot--selected'));
    }
});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function shortId(id) {
    if (!id) return '?';
    // If it looks like "seq-N", return it as-is.
    if (id.startsWith('seq-')) return id;
    // Otherwise, take the last 8 chars of the UUID/hash.
    return id.slice(-8);
}

function formatTs(ts_micros) {
    if (!ts_micros) return '';
    const d = new Date(Math.floor(ts_micros / 1000));
    const pad = n => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
