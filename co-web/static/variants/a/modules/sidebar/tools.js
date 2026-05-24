// ===== CO-280 Phase 1: Tools section =====
// Renders dev/operator tools as a muted bottom-of-sidebar section. Items here
// should be clearly distinct from end-user navigation. The user-reported
// "sidebar.co_dev_ship button is weird" symptom traces back to dev-flavored
// affordances mixed into the same visual treatment as project nav — this
// section gives them a dedicated home.
//
// Phase 1 scope: scaffold the section with the deployments + changelog links
// that already exist as routes. Future phases (CO-280 Phase 2/3) will audit
// individual tool entries (co-auto ship, admin dashboard, etc.) and gate by
// admin role.
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
    // Tools list is intentionally small in Phase 1 — links to existing
    // operator routes. Add to this list as new dev/admin actions emerge.
    const items = [
        { key: 'deployments', label: t('sidebar.tool.deployments') || 'Deployments', href: '/deployments' },
        { key: 'changelog',   label: t('sidebar.tool.changelog')   || 'Changelog',   href: '/changelog' },
    ];
    root.innerHTML = items.map(renderToolItem).join('');
}
