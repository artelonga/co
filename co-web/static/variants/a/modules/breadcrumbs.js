// ===== CO-280: breadcrumbs — platform › universe › sub-universe › project =====
import { state } from './state.js';
import { esc } from './helpers.js';

export function renderBreadcrumbs() {
    const el = document.getElementById('breadcrumbs');
    if (!el) return;

    const slug = state.currentUniverseSlug;

    if (!slug || slug === 'template') {
        el.classList.add('hidden');
        return;
    }

    const info = state.universeInfo;
    const parentKey = info?.parent_key || null;

    const crumbs = [];

    // Platform root — always "CO", links to / (home / template)
    crumbs.push({ label: 'CO', href: '/' });

    if (parentKey) {
        // Sub-universe: resolve parent's display name from any known universe list.
        const allKnown = [
            ...(state.meUniverses?.owned || []),
            ...(state.meUniverses?.member || []),
            ...(state.meUniverses?.subscribed || []),
            ...(state.userUniverses || []),
        ];
        const seen = new Set();
        const parent = allKnown.find(u => {
            if (seen.has(u.key)) return false;
            seen.add(u.key);
            return u.key === parentKey;
        });
        crumbs.push({ label: parent?.name || parentKey, href: `/${parentKey}` });
    }

    // Current universe — link back to universe root when a project is selected
    const hasProject = !!state.currentProject;
    crumbs.push({
        label: info?.name || slug,
        href: hasProject ? `/${slug}` : null,
    });

    // Current project — leaf crumb, no link (you are here)
    if (hasProject) {
        crumbs.push({ label: state.currentProject.name, href: null });
    }

    el.classList.remove('hidden');
    el.innerHTML = crumbs
        .map((c, i) => {
            const sep = i > 0
                ? `<span class="bc-sep" aria-hidden="true">›</span>`
                : '';
            const content = c.href
                ? `<a class="bc-link" href="${esc(c.href)}">${esc(c.label)}</a>`
                : `<span class="bc-current" aria-current="page">${esc(c.label)}</span>`;
            return `${sep}<span class="bc-item">${content}</span>`;
        })
        .join('');
}
