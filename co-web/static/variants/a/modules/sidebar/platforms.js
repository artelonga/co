// ===== CO-280 Phase 1: Platforms section =====
// Renders the hardcoded list of sister deployable units (the 5 platforms that
// make up the ArteLonga stack). External-link icon shown when the URL differs
// from window.location.origin; clicking opens in a new tab in that case.
//
// This is intentionally hardcoded — there is no /platforms HTTP endpoint yet
// (and likely never will be: this list is operator-owned configuration, not
// content). When CO-273 (deployment dashboard) gains a public manifest the
// list can move there; until then a single source of truth here is fine.
import { esc } from '../helpers.js';

export const PLATFORMS = [
    { key: 'co',        name: 'CO',        url: 'https://co.artelonga.com.br' },
    { key: 'artelonga', name: 'ArteLonga', url: 'https://artelonga.com.br' },
    { key: 'quilombo',  name: 'Quilombo',  url: 'https://quilomboaraucaria.org' },
    { key: 'yggdrasil', name: 'Yggdrasil', url: 'https://yggdrasil.artelonga.com.br' },
    { key: 'rfq',       name: 'RFQ',       url: 'https://rfq.fly.dev' },
];

function platformOriginMatches(url) {
    try {
        return new URL(url).origin === window.location.origin;
    } catch (_) {
        return false;
    }
}

function renderPlatformItem(p) {
    const isHere = platformOriginMatches(p.url);
    const target = isHere ? '' : ' target="_blank" rel="noopener"';
    const activeClass = isHere ? ' active' : '';
    const extIcon = isHere
        ? ''
        : '<span class="sidebar-platform-ext" aria-hidden="true">&#8599;</span>';
    return `<a class="sidebar-item sidebar-platform-item${activeClass}"
        href="${esc(p.url)}"
        data-platform="${esc(p.key)}"${target}>
        <span class="sidebar-item-name">${esc(p.name)}</span>
        ${extIcon}
    </a>`;
}

export function renderPlatforms() {
    const root = document.querySelector('#platforms-nav');
    if (!root) return;
    root.innerHTML = PLATFORMS.map(renderPlatformItem).join('');
}
