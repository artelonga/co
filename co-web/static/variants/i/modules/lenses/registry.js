// CO-393: Lens registry — manifest-driven composable lenses.
//
// A lens is { id, label, icon, supports(contentType), render(container, state) }
// where contentType is a manifest content_types[] entry: { name, schema, presentation }.
//
// Lenses self-register on import via registerLens().
// The manifest drives which tabs appear via computeDynamicTabs().

const _registry = [];

export function registerLens(lens) {
    if (!lens || !lens.id) return;
    if (!_registry.find(l => l.id === lens.id)) _registry.push(lens);
}

export function getLenses() { return _registry.slice(); }

export function getLensById(id) { return _registry.find(l => l.id === id) || null; }

// Returns lenses eligible for the given content_type definition.
// contentType: { name, schema, presentation } from manifest.content_types[]
export function getLensesForContentType(contentType) {
    return _registry.filter(l => l.supports(contentType));
}

// Hardcoded base tab IDs — already present in index.html, not injected dynamically.
const BASE_TAB_IDS = new Set(['kanban', 'table', 'timeline', 'calendar', 'dashboard', 'conteudo', 'document', 'changelog', 'workspace']);

// Given a manifest, return additional tabs to inject beyond the static base set.
// Returns array of { id, label, icon, viewKey } — viewKey is the data-view value.
//
// This replaces the gantt-only injectManifestViewTabs from variant a. Any registered
// lens type in manifest.views is handled, not just gantt.
//
// CO-387: named manifest views are generalized — any lens with
// `namedViews: true` (gantt, time-grid, …) gets one tab per named view,
// keyed `<type>:<name>`; renderContent passes the matching view definition
// to lens.render(viewDef).
export function computeDynamicTabs(manifest) {
    const tabs = [];
    const seen = new Set();

    for (const v of (manifest?.views || [])) {
        const type = v.type || v.id || '';
        const lens = getLensById(type);
        if (!lens) continue;

        const viewKey = lens.namedViews ? `${type}:${v.name || 'default'}` : type;
        if (seen.has(viewKey)) continue;
        seen.add(viewKey);

        // Base tabs are already in the HTML — only inject non-base or named views
        if (BASE_TAB_IDS.has(type) && !lens.namedViews) continue;

        tabs.push({
            id: type,
            label: v.label || v.name || lens.label,
            icon: v.icon || lens.icon,
            viewKey,
        });
    }

    // Auto-add non-base lenses supported by declared content_types
    for (const ct of (manifest?.content_types || [])) {
        for (const lens of getLensesForContentType(ct)) {
            if (BASE_TAB_IDS.has(lens.id)) continue;
            if (seen.has(lens.id)) continue;
            seen.add(lens.id);
            tabs.push({ id: lens.id, label: lens.label, icon: lens.icon, viewKey: lens.id });
        }
    }

    return tabs;
}

// Returns true if any content_type in the manifest supports the given lens.
export function manifestSupportsLens(manifest, lensId) {
    const lens = getLensById(lensId);
    if (!lens) return false;
    return (manifest?.content_types || []).some(ct => lens.supports(ct));
}
