// CO-387: <co-time-grid> — the time-rendering primitive's SPA component.
//
// Accepts (entries, lens) and renders. Vanilla JS custom element, no framework.
//
//   <co-time-grid
//     universe="co"                     <- universe key, or comma-list (multi)
//     lens="4-day-week"                 <- optional; comma-list pins stacked lenses
//     range-from="2026-01-01T00:00:00Z" <- optional ISO bounds
//     range-to="2026-12-31T23:59:59Z"
//     group-by="entry_type"
//     view-mode="grid">                 <- grid | timeline | scatter | gantt
//   </co-time-grid>
//
// - Lens list comes from GET /api/v1/universes/{u}/calendar (_calendar.yaml;
//   Gregorian default). Switching lens = re-render only, no API call.
// - Choice persists in localStorage.co_calendar_lens_<universe>.
// - Live updates via the CO-380 event bus (WS /api/v1/events?scope=<u>).
// - Alternatively call el.setData(entries, calendarConfig) to inject data.

import {
    defaultCalendarConfig, resolvedCanonicalField, readCanonicalValue,
    entryToLensPosition, lensPositionToCanonical, formatLensPosition,
    layoutProjectTimeline, entryToTimelineEntry, ganttSpan, MS_PER_DAY,
} from './co-time.js';

const PALETTE = ['#a8d8c8', '#b1c4e8', '#e8c4a3', '#d8a8c8', '#c8e8a8', '#e8a8a8'];
// CO-396: `roadmap` = the project-timeline lane layout (lanes + markers +
// backlog). Lane grouping comes from the lens' `lane_by` or the `lane-by` attr.
const VIEW_MODES = ['grid', 'timeline', 'scatter', 'gantt', 'roadmap'];
const LANE_FIELDS = ['epic', 'module', 'status'];

function t(key, fallback) {
    const v = window.t ? window.t(key) : key;
    return v === key ? fallback : v;
}

function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, c => ({
        '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
    }[c]));
}

function track(name, props) {
    try { if (window.coTrack) window.coTrack(name, props); } catch (_) {}
}

const STYLE = `
:host { display: block; font-family: inherit; color: inherit; }
.ctg-toolbar { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; margin-bottom: 10px; }
.ctg-toolbar select, .ctg-toolbar button {
    font: inherit; font-size: 12px; padding: 4px 10px; border-radius: 6px;
    border: 1px solid rgba(127,127,127,0.4); background: transparent; color: inherit; cursor: pointer;
}
.ctg-toolbar button.active { border-color: currentColor; font-weight: 600; }
.ctg-legend { display: flex; gap: 10px; font-size: 11px; margin-left: auto; }
.ctg-legend span::before { content: '●'; margin-right: 4px; color: var(--dot); }
.ctg-section-title { font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em; opacity: 0.7; margin: 14px 0 6px; }
.ctg-grid { display: grid; gap: 2px; }
.ctg-grid .ctg-head { font-size: 10px; text-transform: uppercase; opacity: 0.6; padding: 2px 4px; }
.ctg-cell { min-height: 38px; border: 1px solid rgba(127,127,127,0.25); border-radius: 4px; padding: 2px 4px; font-size: 11px; cursor: pointer; overflow: hidden; }
.ctg-cell:hover { border-color: currentColor; }
.ctg-cell .ctg-week-label { opacity: 0.5; font-size: 9px; }
.ctg-chip { display: block; border-radius: 3px; padding: 0 3px; margin: 1px 0; background: var(--dot, rgba(127,127,127,0.25)); color: #111; white-space: nowrap; text-overflow: ellipsis; overflow: hidden; }
.ctg-axis { position: relative; height: 90px; border-bottom: 1px solid rgba(127,127,127,0.4); margin: 24px 8px 30px; }
.ctg-dot { position: absolute; bottom: -5px; width: 10px; height: 10px; border-radius: 50%; background: var(--dot, #888); transform: translateX(-50%); cursor: pointer; }
.ctg-dot-label { position: absolute; bottom: 10px; font-size: 10px; transform: translateX(-50%) rotate(-38deg); transform-origin: bottom left; white-space: nowrap; max-width: 140px; overflow: hidden; text-overflow: ellipsis; }
.ctg-marker { position: absolute; top: -18px; font-size: 9px; opacity: 0.6; transform: translateX(-50%); white-space: nowrap; }
.ctg-marker::after { content: ''; position: absolute; left: 50%; top: 14px; height: 80px; border-left: 1px dashed rgba(127,127,127,0.35); }
.ctg-gantt-row { display: flex; align-items: center; gap: 8px; font-size: 11px; margin: 3px 0; }
.ctg-gantt-label { width: 180px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: none; }
.ctg-gantt-track { position: relative; flex: 1; height: 14px; background: rgba(127,127,127,0.12); border-radius: 7px; }
.ctg-gantt-bar { position: absolute; top: 0; height: 100%; border-radius: 7px; background: var(--dot, #888); min-width: 6px; }
.ctg-empty { padding: 24px; text-align: center; opacity: 0.6; font-size: 13px; }
.ctg-orphans { font-size: 10px; opacity: 0.5; margin-top: 6px; }
.ctg-break { opacity: 0.45; }
/* CO-396 project-timeline (roadmap) */
.ctg-lane { display: flex; align-items: stretch; gap: 8px; margin: 2px 0; }
.ctg-lane-label { width: 140px; flex: none; font-size: 11px; font-weight: 600; padding: 4px 6px; opacity: 0.85; align-self: center; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.ctg-lane-track { position: relative; flex: 1; min-height: 22px; padding: 2px 0; border-top: 1px solid rgba(127,127,127,0.15); }
.ctg-bar { position: absolute; height: 14px; border-radius: 7px; background: var(--dot, #888); min-width: 6px; box-sizing: border-box; cursor: default; color: #111; font-size: 10px; line-height: 14px; padding: 0 5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.ctg-bar.ctg-point { width: 14px; min-width: 14px; border-radius: 50%; padding: 0; }
.ctg-roadmap-axis { position: relative; height: 18px; margin-left: 148px; border-bottom: 1px dashed rgba(127,127,127,0.4); }
.ctg-roadmap-marker { position: absolute; top: 0; font-size: 9px; opacity: 0.8; transform: translateX(-50%); white-space: nowrap; }
.ctg-roadmap-marker::after { content: ''; position: absolute; left: 50%; top: 12px; height: 1000px; border-left: 1px dashed rgba(180,120,80,0.45); z-index: 0; }
.ctg-roadmap-marker.ctg-release { color: #b9742c; font-weight: 600; }
.ctg-backlog { margin-top: 14px; padding: 8px; border: 1px dashed rgba(127,127,127,0.35); border-radius: 6px; }
.ctg-backlog-title { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; opacity: 0.6; margin-bottom: 6px; }
.ctg-backlog-chip { display: inline-block; border-radius: 3px; padding: 1px 6px; margin: 2px; font-size: 11px; background: rgba(127,127,127,0.18); }
`;

class CoTimeGrid extends HTMLElement {
    constructor() {
        super();
        this.attachShadow({ mode: 'open' });
        this._entries = [];
        this._config = null;
        this._sockets = [];
        this._reloadTimer = null;
        this._externalData = false;
    }

    static get observedAttributes() { return ['universe', 'lens', 'view-mode', 'range-from', 'range-to']; }

    connectedCallback() {
        if (!this._externalData) this._load();
        else this._render();
    }

    disconnectedCallback() { this._closeSockets(); }

    attributeChangedCallback(name, oldV, newV) {
        if (!this.isConnected || oldV === newV) return;
        // universe changes need a refetch; lens / view-mode / range are local.
        if (name === 'universe' && !this._externalData) this._load();
        else this._render();
    }

    // Public API: inject entries + calendar config directly (no fetching).
    setData(entries, config) {
        this._externalData = true;
        this._entries = entries || [];
        this._config = config || defaultCalendarConfig();
        if (this.isConnected) this._render();
    }

    get _universes() {
        return (this.getAttribute('universe') || '').split(',').map(s => s.trim()).filter(Boolean);
    }

    get _viewMode() {
        const m = this.getAttribute('view-mode');
        return VIEW_MODES.includes(m) ? m : 'grid';
    }

    // CO-396: lane grouping for the roadmap (project-timeline) mode.
    // Priority: the `lane-by` attribute → the active lens' `lane_by` → status.
    _laneBy(lens) {
        const attr = (this.getAttribute('lane-by') || '').trim().toLowerCase();
        if (LANE_FIELDS.includes(attr)) return attr;
        if (lens && LANE_FIELDS.includes(lens.lane_by)) return lens.lane_by;
        return 'status';
    }

    // Pinned lens list from the attribute (?lens=A,B,C stacking).
    get _pinnedLenses() {
        return (this.getAttribute('lens') || '').split(',').map(s => s.trim()).filter(Boolean);
    }

    get _storageKey() { return 'co_calendar_lens_' + (this._universes[0] || 'template'); }

    _activeLensIds() {
        const pinned = this._pinnedLenses;
        if (pinned.length > 0) return pinned;
        try {
            const stored = localStorage.getItem(this._storageKey);
            if (stored && this._config && this._config.lenses.some(l => l.id === stored)) return [stored];
        } catch (_) {}
        return [(this._config && this._config.default_lens) || 'gregorian'];
    }

    async _load() {
        const universes = this._universes;
        if (universes.length === 0) { this._config = defaultCalendarConfig(); this._render(); return; }
        try {
            // Merge calendar configs (first universe wins on id collision + default).
            const configs = await Promise.all(universes.map(u =>
                fetch(`/api/v1/universes/${encodeURIComponent(u)}/calendar`).then(r => r.ok ? r.json() : null).catch(() => null)));
            const lenses = [];
            const seen = new Set();
            for (const cfg of configs) {
                for (const lens of (cfg && cfg.lenses) || []) {
                    if (!seen.has(lens.id)) { seen.add(lens.id); lenses.push(lens); }
                }
            }
            this._config = lenses.length > 0
                ? { default_lens: (configs.find(c => c) || {}).default_lens || lenses[0].id, lenses }
                : defaultCalendarConfig();

            // One broad entries fetch per universe — lens switching then
            // re-renders locally with no further API calls.
            const lists = await Promise.all(universes.map(u =>
                fetch(`/api/v1/universes/${encodeURIComponent(u)}/entries?limit=1000`)
                    .then(r => r.ok ? r.json() : null).catch(() => null)));
            this._entries = [];
            lists.forEach((res, i) => {
                for (const e of (res && res.entries) || []) {
                    this._entries.push({ ...e, _universe: universes[i] });
                }
            });
        } catch (e) {
            console.warn('co-time-grid: load failed', e);
            this._config = this._config || defaultCalendarConfig();
        }
        this._render();
        this._openSockets();
    }

    // ---- CO-380 live updates -------------------------------------------------
    _openSockets() {
        this._closeSockets();
        if (this.getAttribute('live') === 'false') return;
        const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
        for (const u of this._universes) {
            try {
                const ws = new WebSocket(`${proto}//${location.host}/api/v1/events?scope=${encodeURIComponent(u)}`);
                ws.onmessage = (msg) => {
                    try {
                        const ev = JSON.parse(msg.data);
                        if (ev && typeof ev.event_type === 'string' && ev.event_type.startsWith('entry.')) {
                            this._scheduleReload();
                        }
                    } catch (_) {}
                };
                this._sockets.push(ws);
            } catch (_) {}
        }
    }

    _closeSockets() {
        for (const ws of this._sockets) { try { ws.close(); } catch (_) {} }
        this._sockets = [];
    }

    _scheduleReload() {
        // Debounce bursts; stays well under the 300ms live-update budget.
        clearTimeout(this._reloadTimer);
        this._reloadTimer = setTimeout(async () => {
            const universes = this._universes;
            const lists = await Promise.all(universes.map(u =>
                fetch(`/api/v1/universes/${encodeURIComponent(u)}/entries?limit=1000`)
                    .then(r => r.ok ? r.json() : null).catch(() => null)));
            this._entries = [];
            lists.forEach((res, i) => {
                for (const e of (res && res.entries) || []) {
                    this._entries.push({ ...e, _universe: universes[i] });
                }
            });
            this._render();
        }, 100);
    }

    // ---- Rendering -----------------------------------------------------------
    _inRange(ms) {
        const from = Date.parse(this.getAttribute('range-from') || '');
        const to = Date.parse(this.getAttribute('range-to') || '');
        if (!Number.isNaN(from) && ms < from) return false;
        if (!Number.isNaN(to) && ms > to) return false;
        return true;
    }

    _items(lens) {
        const items = [];
        let orphans = 0;
        const isMs = !!{ event_at_ms: 1, due_at_ms: 1, scheduled_at_ms: 1 }[resolvedCanonicalField(lens)];
        for (const e of this._entries) {
            const raw = readCanonicalValue(e, lens);
            if (raw === null) continue;
            if (isMs && !this._inRange(raw)) continue;
            const pos = entryToLensPosition(raw, lens);
            if (!pos) { orphans++; continue; }
            items.push({ entry: e, raw, pos });
        }
        return { items, orphans };
    }

    _color(entry) {
        const i = Math.max(this._universes.indexOf(entry._universe), 0);
        return PALETTE[i % PALETTE.length];
    }

    _render() {
        const cfg = this._config || defaultCalendarConfig();
        const lensIds = this._activeLensIds();
        const pinned = this._pinnedLenses.length > 0;
        const root = this.shadowRoot;
        root.innerHTML = `<style>${STYLE}</style>`;

        // Toolbar: lens picker (single mode) + view-mode tabs + legend.
        const bar = document.createElement('div');
        bar.className = 'ctg-toolbar';
        if (!pinned) {
            const sel = document.createElement('select');
            sel.title = t('time.lens', 'Lente');
            sel.setAttribute('aria-label', t('time.lens', 'Lente'));
            for (const lens of cfg.lenses) {
                const opt = document.createElement('option');
                opt.value = lens.id;
                opt.textContent = '🗓 ' + lens.name;
                if (lens.id === lensIds[0]) opt.selected = true;
                sel.appendChild(opt);
            }
            sel.addEventListener('change', () => {
                const from = lensIds[0];
                try { localStorage.setItem(this._storageKey, sel.value); } catch (_) {}
                track('time.lens_switched', { universe: this._universes[0], from_lens: from, to_lens: sel.value });
                this._render(); // re-render only — no API call
            });
            bar.appendChild(sel);
        }
        for (const mode of VIEW_MODES) {
            const btn = document.createElement('button');
            btn.textContent = t('time.view.' + mode, mode);
            if (mode === this._viewMode) btn.classList.add('active');
            btn.addEventListener('click', () => {
                if (this.getAttribute('view-mode') !== mode) this.setAttribute('view-mode', mode);
            });
            bar.appendChild(btn);
        }
        if (this._universes.length > 1) {
            const legend = document.createElement('div');
            legend.className = 'ctg-legend';
            this._universes.forEach((u, i) => {
                const s = document.createElement('span');
                s.style.setProperty('--dot', PALETTE[i % PALETTE.length]);
                s.textContent = u;
                legend.appendChild(s);
            });
            bar.appendChild(legend);
        }
        root.appendChild(bar);

        // One stacked section per active lens (?lens=A,B,C).
        for (const lensId of lensIds) {
            const lens = cfg.lenses.find(l => l.id === lensId);
            const section = document.createElement('div');
            if (!lens) {
                section.innerHTML = `<div class="ctg-empty">? ${esc(lensId)}</div>`;
                root.appendChild(section);
                continue;
            }
            if (lensIds.length > 1) {
                const h = document.createElement('div');
                h.className = 'ctg-section-title';
                h.textContent = lens.name;
                section.appendChild(h);
            }
            // CO-396: roadmap mode reads dates itself (lanes/markers/backlog)
            // and must render even when no entry has a positionable date — the
            // backlog lane is the whole point ("nada some").
            if (this._viewMode === 'roadmap') {
                this._renderProjectTimeline(section, lens);
                root.appendChild(section);
                continue;
            }
            const { items, orphans } = this._items(lens);
            track('time.grid_rendered', { universe: this._universes.join(','), lens: lens.id, entry_count: items.length });
            if (items.length === 0) {
                section.insertAdjacentHTML('beforeend',
                    `<div class="ctg-empty">${esc(t('time.no_entries', 'Nenhuma entrada com data neste intervalo'))}</div>`);
                root.appendChild(section);
                continue;
            }
            const mode = this._viewMode;
            const linearGrid = items[0].pos.kind === 'linear' || items[0].pos.kind === 'cell';
            if (mode === 'grid' && linearGrid) this._renderGrid(section, lens, items);
            else if (mode === 'gantt') this._renderGantt(section, lens, items);
            else if (mode === 'scatter') this._renderAxis(section, lens, items, true);
            else this._renderAxis(section, lens, items, false);
            if (orphans > 0) {
                section.insertAdjacentHTML('beforeend', `<div class="ctg-orphans">+${orphans} orphan(s)</div>`);
            }
            root.appendChild(section);
        }
    }

    // grid: weeks × days (linear) or sequential cells (Pomodoro).
    _renderGrid(section, lens, items) {
        if (items[0].pos.kind === 'cell') {
            const byCell = new Map();
            for (const it of items) {
                const k = it.pos.cell;
                if (!byCell.has(k)) byCell.set(k, []);
                byCell.get(k).push(it);
            }
            const grid = document.createElement('div');
            grid.className = 'ctg-grid';
            grid.style.gridTemplateColumns = 'repeat(auto-fill, minmax(110px, 1fr))';
            const cells = [...byCell.keys()].sort((a, b) => a - b);
            for (const c of cells) {
                const cell = document.createElement('div');
                cell.className = 'ctg-cell';
                cell.innerHTML = `<span class="ctg-week-label">${esc(formatLensPosition(byCell.get(c)[0].pos, lens))}</span>`;
                for (const it of byCell.get(c)) {
                    cell.insertAdjacentHTML('beforeend',
                        `<span class="ctg-chip ${it.pos.in_break ? 'ctg-break' : ''}" style="--dot:${this._color(it.entry)}" title="${esc(it.entry.title)}">${esc(it.entry.title || it.entry.path)}</span>`);
                }
                grid.appendChild(cell);
            }
            section.appendChild(grid);
            return;
        }

        const weekLen = Math.max(lens.week_length_days || 7, 1);
        const weeks = items.map(i => i.pos.week);
        const minW = Math.min(...weeks);
        const maxW = Math.min(Math.max(...weeks), minW + 120); // cap rendered rows
        const byKey = new Map();
        for (const it of items) {
            const k = it.pos.week + ':' + it.pos.day_of_week;
            if (!byKey.has(k)) byKey.set(k, []);
            byKey.get(k).push(it);
        }
        const grid = document.createElement('div');
        grid.className = 'ctg-grid';
        grid.style.gridTemplateColumns = `54px repeat(${weekLen}, 1fr)`;
        grid.insertAdjacentHTML('beforeend', '<div class="ctg-head"></div>');
        for (let d = 0; d < weekLen; d++) {
            const name = (lens.weekday_names || [])[d] || ('D' + (d + 1));
            grid.insertAdjacentHTML('beforeend', `<div class="ctg-head">${esc(name)}</div>`);
        }
        for (let w = minW; w <= maxW; w++) {
            grid.insertAdjacentHTML('beforeend',
                `<div class="ctg-head">${esc(t('time.week', 'Semana'))} ${w}</div>`);
            for (let d = 0; d < weekLen; d++) {
                const cell = document.createElement('div');
                cell.className = 'ctg-cell';
                cell.addEventListener('click', (ev) => {
                    if (ev.target !== cell) return;
                    const canonical = lensPositionToCanonical({ kind: 'linear', week: w, day_of_week: d, hour: 0 }, lens);
                    this.dispatchEvent(new CustomEvent('co-time-grid:create', {
                        detail: { canonical, field: resolvedCanonicalField(lens), lens: lens.id },
                        bubbles: true, composed: true,
                    }));
                });
                for (const it of byKey.get(w + ':' + d) || []) {
                    cell.insertAdjacentHTML('beforeend',
                        `<span class="ctg-chip" style="--dot:${this._color(it.entry)}" title="${esc(it.entry.title)}">${esc(it.entry.title || it.entry.path)}</span>`);
                }
                grid.appendChild(cell);
            }
        }
        section.appendChild(grid);
    }

    // timeline + scatter: normalized horizontal axis. Log lenses place the
    // deep past (Big Bang) at the left edge, the present at the right.
    _renderAxis(section, lens, items, jitter) {
        const coord = (pos, raw) => pos.kind === 'log' ? pos.log_position
            : pos.kind === 'axis' ? pos.position : raw;
        const vals = items.map(it => coord(it.pos, it.raw));
        const markers = (lens.label_periods || []).map(p => ({
            name: p.name,
            v: (lens.scale === 'log') ? Math.log10(Math.max(p.at, 1)) : p.at,
        }));
        let min = Math.min(...vals, ...markers.map(m => m.v));
        let max = Math.max(...vals, ...markers.map(m => m.v));
        if (min === max) { min -= 1; max += 1; }
        const pct = v => {
            const f = (v - min) / (max - min);
            return (lens.scale === 'log' ? (1 - f) : f) * 96 + 2; // log: past left → present right
        };
        const axis = document.createElement('div');
        axis.className = 'ctg-axis';
        for (const m of markers) {
            axis.insertAdjacentHTML('beforeend',
                `<div class="ctg-marker" style="left:${pct(m.v)}%">${esc(m.name)}</div>`);
        }
        items.forEach((it, i) => {
            const left = pct(coord(it.pos, it.raw));
            const lift = jitter ? (i % 5) * 14 : 0;
            axis.insertAdjacentHTML('beforeend',
                `<div class="ctg-dot" style="left:${left}%;bottom:${-5 + lift}px;--dot:${this._color(it.entry)}" title="${esc(it.entry.title)} — ${esc(formatLensPosition(it.pos, lens))}"></div>` +
                (jitter ? '' : `<div class="ctg-dot-label" style="left:${left}%">${esc(it.entry.title || it.entry.path)}</div>`));
        });
        section.appendChild(axis);
    }

    // gantt: bar per entry. CO-396: duration is the shared engine's span —
    // scheduled→due, else created→done, else a point — not just event→due.
    _renderGantt(section, lens, items) {
        const spans = items.map(it => {
            const span = ganttSpan(entryToTimelineEntry(it.entry));
            const start = span ? span.start_ms : it.raw;
            const end = span ? span.end_ms : it.raw;
            return { it, start, end };
        });
        let min = Math.min(...spans.map(s => s.start));
        let max = Math.max(...spans.map(s => s.end));
        if (min === max) { min -= 1; max += 1; }
        const pct = v => ((v - min) / (max - min)) * 100;
        const wrap = document.createElement('div');
        for (const s of spans) {
            wrap.insertAdjacentHTML('beforeend',
                `<div class="ctg-gantt-row">
                    <span class="ctg-gantt-label" title="${esc(s.it.entry.title)}">${esc(s.it.entry.title || s.it.entry.path)}</span>
                    <span class="ctg-gantt-track"><span class="ctg-gantt-bar" style="left:${pct(s.start)}%;width:${Math.max(pct(s.end) - pct(s.start), 0.5)}%;--dot:${this._color(s.it.entry)}"></span></span>
                </div>`);
        }
        section.appendChild(wrap);
    }

    // CO-396: project-timeline (roadmap) — the shared engine's lanes + markers
    // + backlog. Lanes are grouped by epic/module/status; release/milestone
    // entries become vertical markers; undated entries sit in the backlog.
    _renderProjectTimeline(section, lens) {
        const groupBy = this._laneBy(lens);
        const tl = layoutProjectTimeline(this._entries, groupBy);
        track('time.roadmap_rendered', {
            universe: this._universes.join(','), group_by: groupBy,
            lanes: tl.lanes.length, markers: tl.markers.length, backlog: tl.backlog.length,
        });

        if (tl.lanes.length === 0 && tl.markers.length === 0 && tl.backlog.length === 0) {
            section.insertAdjacentHTML('beforeend',
                `<div class="ctg-empty">${esc(t('time.no_entries', 'Nenhuma entrada com data neste intervalo'))}</div>`);
            return;
        }

        // Shared time axis across every dated item + marker.
        const ends = [];
        for (const lane of tl.lanes) for (const it of lane.items) ends.push(it.span.start_ms, it.span.end_ms);
        for (const m of tl.markers) ends.push(m.at_ms);
        let min = ends.length ? Math.min(...ends) : 0;
        let max = ends.length ? Math.max(...ends) : 1;
        if (min === max) { min -= MS_PER_DAY; max += MS_PER_DAY; }
        const pct = v => ((v - min) / (max - min)) * 100;

        // Marker rail (releases/milestones) above the lanes.
        if (tl.markers.length) {
            const axis = document.createElement('div');
            axis.className = 'ctg-roadmap-axis';
            for (const m of tl.markers) {
                const cls = m.kind === 'release' ? 'ctg-release' : '';
                axis.insertAdjacentHTML('beforeend',
                    `<div class="ctg-roadmap-marker ${cls}" style="left:${pct(m.at_ms)}%" title="${esc(m.title)} (${esc(m.kind)})">◆ ${esc(m.title)}</div>`);
            }
            section.appendChild(axis);
        }

        // One row per lane; bars positioned on the shared axis.
        let laneColor = 0;
        for (const lane of tl.lanes) {
            const color = PALETTE[laneColor++ % PALETTE.length];
            const row = document.createElement('div');
            row.className = 'ctg-lane';
            const label = lane.label || t('time.ungrouped', '(sem grupo)');
            row.insertAdjacentHTML('beforeend',
                `<div class="ctg-lane-label" title="${esc(label)}">${esc(label)}</div>`);
            const track = document.createElement('div');
            track.className = 'ctg-lane-track';
            let stack = 0;
            for (const it of lane.items) {
                const isPoint = it.span.start_ms === it.span.end_ms;
                const left = pct(it.span.start_ms);
                const width = Math.max(pct(it.span.end_ms) - left, 0.4);
                const top = (stack++ % 3) * 16;
                if (isPoint) {
                    track.insertAdjacentHTML('beforeend',
                        `<span class="ctg-bar ctg-point" style="left:${left}%;top:${top}px;--dot:${color}" title="${esc(it.title)}"></span>`);
                } else {
                    track.insertAdjacentHTML('beforeend',
                        `<span class="ctg-bar" style="left:${left}%;width:${width}%;top:${top}px;--dot:${color}" title="${esc(it.title)}">${esc(it.title)}</span>`);
                }
            }
            track.style.minHeight = (Math.min(lane.items.length, 3) * 16 + 6) + 'px';
            row.appendChild(track);
            section.appendChild(row);
        }

        // Backlog lane on the margin — undated entries never vanish.
        if (tl.backlog.length) {
            const box = document.createElement('div');
            box.className = 'ctg-backlog';
            box.insertAdjacentHTML('beforeend',
                `<div class="ctg-backlog-title">${esc(t('time.backlog', 'Sem data'))} (${tl.backlog.length})</div>`);
            for (const b of tl.backlog) {
                box.insertAdjacentHTML('beforeend',
                    `<span class="ctg-backlog-chip" title="${esc(b.title)}">${esc(b.title || b.id)}</span>`);
            }
            section.appendChild(box);
        }
    }
}

if (!customElements.get('co-time-grid')) {
    customElements.define('co-time-grid', CoTimeGrid);
}

export { CoTimeGrid };
