// ===== View-specific state defaults =====
// Tasks that add new view state fields (kanban, table, timeline, calendar)
// should add them here so parallel changes don't conflict with universe/auth work.

export function createViewDefaults() {
    return {
        view: 'conteudo',
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
