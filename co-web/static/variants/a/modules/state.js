// ===== Shared mutable application state =====
// All modules import this object and mutate it in place.
// ES modules share the same binding, so mutations are visible across modules.

export const state = {
    // Current authenticated user (null = anonymous).
    me: null,
    projects: [],
    currentProject: null,
    tasks: [],
    view: 'conteudo',
    editingTaskId: null,
    searchQuery: '',
    loading: false,
    showArchived: false,
    userUniverses: [],
    // CO-191: bucketed universes shape from /api/v1/me/universes.
    // null = not yet loaded (anonymous or pre-login).
    meUniverses: null,
    // 1.62.0 Phase 7: per-slug pin map. When set, the SPA appends
    // `?as_of=<pin>` to entry queries so the user sees the rewind view.
    // Populated on universe-info open and after pin/unpin actions.
    subscriptionPin: {},
    // Calendar
    calendarDate: new Date(),
    // Table
    sortColumn: 'key',
    sortDirection: 'asc',
    selectedIds: new Set(),
    collapsedGroups: new Set(),
    openStatusDropdown: null,
    // Timeline
    zoom: 'month',
    timelineStart: null,
    miniCalDate: new Date(),
    collapsedSwimlanes: {},
    unscheduledCollapsed: false,
    // Subtree expand/collapse (shared across views)
    collapsedSubtasks: new Set(),
    // Universe routing
    currentUniverseSlug: 'template',
    isTemplate: false,
    universeInfo: null,
    // Form config (CO-24): theme, layout, fonts from universe config endpoint
    universeConfig: null,
    // CO-38: Yggdrasil minigames hub
    isYggdrasil: false,
    gameView: null, // active game slug, e.g. 'tetris'
    // CO-73: universe manifest (loaded from /api/v1/universes/:slug/manifest)
    universeManifest: null,
    // CO-73: entries fetched for calendar/gantt views
    calendarEntries: [],
    // Universe switch guard
    switchingUniverse: false,
};

// CO-253: Returns true if the current user can edit the current universe.
// Template universe always returns true (handled by the clone/login flow).
// Non-template: requires user to be owner, member, or subscriber.
export function canEditCurrentUniverse() {
    if (state.isTemplate) return true;
    if (!state.me) return false;
    if (!state.meUniverses) return false;
    const slug = state.currentUniverseSlug;
    const buckets = [
        ...(state.meUniverses.owned || []),
        ...(state.meUniverses.member || []),
        ...(state.meUniverses.subscribed || []),
    ];
    return buckets.some(u => (u.key ?? u.universe?.key) === slug);
}
