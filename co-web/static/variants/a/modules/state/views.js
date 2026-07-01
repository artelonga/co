// ===== View-specific state defaults =====
// Tasks that add new view state fields (kanban, table, timeline, calendar)
// should add them here so parallel changes don't conflict with universe/auth work.

// CO-558: allow a board view to be deep-linked via `?view=<name>` so the
// standalone /changelog route can redirect into the single board changelog
// surface (`?view=changelog`) instead of being a second changelog page.
const DEEP_LINKABLE_VIEWS = new Set([
    'conteudo', 'kanban', 'table', 'calendar', 'timeline',
    'dashboard', 'changelog', 'workspace', 'scrum',
]);

function initialViewFromUrl() {
    try {
        const requested = new URLSearchParams(window.location.search).get('view');
        if (requested && DEEP_LINKABLE_VIEWS.has(requested)) return requested;
    } catch (_) { /* non-browser / no location */ }
    return 'conteudo';
}

export function createViewDefaults() {
    return {
        view: initialViewFromUrl(),
        calendarDate: new Date(),
        sortColumn: 'key',
        sortDirection: 'asc',
        selectedIds: new Set(),
        collapsedGroups: new Set(),
        openStatusDropdown: null,
        zoom: 'month',
        timelineStart: null,
        miniCalDate: new Date(),
        collapsedSwimlanes: {},
        unscheduledCollapsed: false,
        collapsedSubtasks: new Set(),
    };
}
