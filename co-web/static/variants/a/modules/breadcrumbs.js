// ===== CO-280: breadcrumbs — platform › universe › sub-universe › project =====
// CO-358: at ≤480px collapse to ← <title> with tap-to-show full trail popover.
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

    // Current universe — link back when a project is selected
    const hasProject = !!state.currentProject;
    crumbs.push({
        label: info?.name || slug,
        href: hasProject ? `/${slug}` : null,
    });

    // Current project — leaf crumb, no link
    if (hasProject) {
        crumbs.push({ label: state.currentProject.name, href: null });
    }

    el.classList.remove('hidden');

    // The "current" label is the last crumb
    const currentLabel = crumbs[crumbs.length - 1].label;

    // Mobile back: parent href is the second-to-last crumb with a href, or '/'
    const parentCrumb = [...crumbs].reverse().find((c, i) => i > 0 && c.href);
    const backHref = parentCrumb?.href || '/';

    // Full trail HTML (desktop, always rendered but hidden on ≤480px via CSS)
    const fullTrail = crumbs
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

    // Mobile popover trail items
    const popoverItems = crumbs
        .map(c => c.href
            ? `<a class="bc-trail-item" href="${esc(c.href)}">${esc(c.label)}</a>`
            : `<span class="bc-trail-item bc-trail-current">${esc(c.label)}</span>`)
        .join('');

    el.innerHTML = `
        ${fullTrail}
        <span class="bc-mobile-back" role="button" tabindex="0" aria-label="Abrir trilha de navegação">
            <a class="bc-back-arrow" href="${esc(backHref)}" aria-label="Voltar">←</a>
            <span class="bc-back-title" id="bc-title-trigger">${esc(currentLabel)}</span>
        </span>
        <div class="bc-trail-popover hidden" id="bc-trail-popover" role="dialog" aria-label="Navegação estrutural">
            <div class="bc-trail-close">
                <button class="bc-trail-close-btn" id="bc-trail-close-btn" aria-label="Fechar">×</button>
            </div>
            ${popoverItems}
        </div>`;

    // Wire up the title tap → open popover
    const titleTrigger = el.querySelector('#bc-title-trigger');
    const popover = el.querySelector('#bc-trail-popover');
    const closeBtn = el.querySelector('#bc-trail-close-btn');

    if (titleTrigger && popover) {
        titleTrigger.addEventListener('click', (e) => {
            e.stopPropagation();
            popover.classList.toggle('hidden');
        });
        titleTrigger.addEventListener('keydown', (e) => {
            if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                popover.classList.toggle('hidden');
            }
        });
    }

    if (closeBtn && popover) {
        closeBtn.addEventListener('click', () => popover.classList.add('hidden'));
    }

    ensureOutsideClickClose();
}

// Singleton outside-click closer — renderBreadcrumbs() runs on every render,
// so the handler must NOT be re-registered per render (it would leak one
// closure per render onto document). Queries the live DOM instead of
// capturing the popover node.
let _outsideCloseWired = false;
function ensureOutsideClickClose() {
    if (_outsideCloseWired) return;
    _outsideCloseWired = true;
    document.addEventListener('click', (e) => {
        const popover = document.getElementById('bc-trail-popover');
        const root = document.getElementById('breadcrumbs');
        if (popover && !popover.classList.contains('hidden') && root && !root.contains(e.target)) {
            popover.classList.add('hidden');
        }
    });
}
