// ===== Sidebar tree-building and section HTML =====
import { state } from '../state.js';
import { esc } from '../helpers.js';

export function buildChildMap(universes, allUniversesMap = null) {
    const keys = new Set(universes.map(u => u.key));
    const childrenByParent = {};
    const topLevel = [];
    const syntheticByKey = {};
    universes.forEach(u => {
        if (u.parent_key && keys.has(u.parent_key)) {
            (childrenByParent[u.parent_key] = childrenByParent[u.parent_key] || []).push(u);
        } else if (u.parent_key && allUniversesMap && allUniversesMap.has(u.parent_key)) {
            const pk = u.parent_key;
            if (!syntheticByKey[pk]) {
                syntheticByKey[pk] = { ...allUniversesMap.get(pk), _synthetic: true };
                topLevel.push(syntheticByKey[pk]);
            }
            (childrenByParent[pk] = childrenByParent[pk] || []).push(u);
        } else {
            topLevel.push(u);
        }
    });
    return { childrenByParent, topLevel };
}

export function renderUniverseItemHtml(u, childrenByParent, depth, showRoleChip) {
    const active = u.key === state.currentUniverseSlug ? ' active' : '';
    const kids = childrenByParent[u.key];
    const hasKids = !!(kids && kids.length);
    const expandKey = `co_universe_tree_${u.key}`;
    const stored = localStorage.getItem(expandKey);
    // Default-expand the tree when EITHER the current universe is this
    // parent (so a user on "tempo" sees its subuniverses below) OR the
    // current universe is one of the descendants. Previously only the
    // descendant case was handled, so navigating to the parent left its
    // children collapsed and looking absent.
    const isSelfActive = u.key === state.currentUniverseSlug;
    const containsActive = hasKids && kids.some(k => k.key === state.currentUniverseSlug);
    const expanded = stored !== null
        ? (stored === '1')
        : (isSelfActive || containsActive);
    const indent = 12 + depth * 16;
    const chevron = hasKids
        ? `<span class="sidebar-universe-chevron" data-toggle="${esc(u.key)}" style="display:inline-block;width:14px;text-align:center;cursor:pointer;user-select:none">${expanded ? '▾' : '▸'}</span>`
        : '<span class="sidebar-universe-chevron-spacer" style="display:inline-block;width:14px"></span>';
    // CO-319: helper that treats "key returned as-is" (untranslated) as falsy,
    // so the fallback actually fires when a translation is missing. The old
    // `t(k) || fallback` pattern silently rendered the raw key.
    const tOr = (key, fallback) => {
        if (!window.t) return fallback;
        const v = window.t(key);
        return (!v || v === key) ? fallback : v;
    };
    const role = u.role;
    const roleChip = showRoleChip && role && !u._synthetic
        ? `<span class="role-chip">${esc(tOr('sidebar.role.' + role, role))}</span>`
        : '';
    // CO-319: oss-chip removed — it was decorative ("código aberto" badge on
    // the co universe) and its missing translation caused the raw key
    // `sidebar.co_dev_chip` to render in the sidebar (user reported "weird"
    // CO row). The universe name + role chip carry enough information.
    const subCount = hasKids ? ` (${kids.length})` : '';
    const syntheticClass = u._synthetic ? ' sidebar-universe-synthetic' : '';
    let html = `<div class="sidebar-item sidebar-universe-item${syntheticClass}${active}" data-universe="${esc(u.key)}" style="padding-left:${indent}px">
        ${chevron}<span class="sidebar-item-name">${esc(u.name || u.key)}${subCount}</span>${roleChip}
    </div>`;
    if (hasKids && expanded) {
        for (const k of kids) html += renderUniverseItemHtml(k, childrenByParent, depth + 1, showRoleChip);
    }
    return html;
}

export function renderSectionHtml(label, universes, showRoleChip, tooltip = '', allUniversesMap = null) {
    if (!universes || universes.length === 0) return '';
    const { childrenByParent, topLevel } = buildChildMap(universes, allUniversesMap);
    const titleAttr = tooltip ? ` title="${esc(tooltip)}"` : '';
    return `<div class="sidebar-universe-section">
        <div class="sidebar-section-label"${titleAttr}>${esc(label)}</div>
        ${topLevel.map(u => renderUniverseItemHtml(u, childrenByParent, 0, showRoleChip)).join('')}
    </div>`;
}

export function renderInviteRowHtml(inv) {
    const t = window.t || (k => k);
    return `<div class="sidebar-invite-row">
        <div class="sidebar-item sidebar-universe-item" style="padding-left:12px">
            <span class="sidebar-universe-chevron-spacer" style="display:inline-block;width:14px"></span>
            <span class="sidebar-item-name">${esc(inv.universe_name)}</span>
        </div>
        <div class="invite-row-actions">
            <button class="btn btn-sm btn-primary invite-accept-btn" data-key="${esc(inv.universe_key)}">${esc(t('sidebar.invite.accept'))}</button>
            <button class="btn btn-sm btn-ghost invite-decline-btn" data-key="${esc(inv.universe_key)}">${esc(t('sidebar.invite.decline'))}</button>
        </div>
    </div>`;
}

export function renderDiscoverableItemHtml(u) {
    const t = window.t || (k => k);
    return `<div class="sidebar-item sidebar-discoverable-item" data-universe="${esc(u.key)}" style="padding-left:12px">
        <span class="sidebar-universe-chevron-spacer" style="display:inline-block;width:14px"></span>
        <span class="sidebar-item-name">${esc(u.name || u.key)}</span>
        <button class="btn btn-sm btn-ghost discover-subscribe-btn" data-key="${esc(u.key)}" style="margin-left:auto;font-size:11px">${esc(t('sidebar.discover.subscribe'))}</button>
    </div>`;
}
