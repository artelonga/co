// ===== CO-280 Phase 1: Gestão (management) section — CO-559 =====
// Renders dev/operator tools as a muted bottom-of-sidebar section. Items here
// should be clearly distinct from end-user navigation. The user-reported
// "sidebar.co_dev_ship button is weird" symptom traces back to dev-flavored
// affordances mixed into the same visual treatment as project nav — this
// section gives them a dedicated home.
//
// Phase 1 scope: scaffold the section with the deployments link that already
// exists as a route. Future phases (CO-280 Phase 2/3) will audit individual
// entries (co-auto ship, admin dashboard, etc.) and gate by admin role.
// CO-559: this is the "Gestão"/"Management" section (admin actions), not the
// CO-503 on-demand "Tools" surface. CO-558: the changelog was removed from this
// list — it lives only on the board's Histórico view-tab.
import { esc } from '../helpers.js';

function renderToolItem(t) {
    const target = t.external ? ' target="_blank" rel="noopener"' : '';
    return `<a class="sidebar-item sidebar-tool-item"
        href="${esc(t.href)}"
        data-tool="${esc(t.key)}"${target}>
        <span class="sidebar-item-name">${esc(t.label)}</span>
    </a>`;
}

export function renderTools() {
    const root = document.querySelector('#tools-nav');
    if (!root) return;
    const t = window.t || (k => k);
    // CO-559: this is the "Gestão"/"Management" section — admin/operator actions,
    // not CO-503 on-demand tools. CO-558: the changelog is not listed here; it has
    // a single home as the board's Histórico view-tab (the standalone /changelog
    // route redirects there), so Gestão holds only Deployments.
    const items = [
        { key: 'deployments', label: t('sidebar.tool.deployments') || 'Deployments', href: '/deployments' },
    ];
    root.innerHTML = items.map(renderToolItem).join('');
}
