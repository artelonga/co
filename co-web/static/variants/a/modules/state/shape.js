// ===== Shared mutable application state =====
// All modules import this object and mutate it in place.
// ES modules share the same binding, so mutations are visible across modules.
import { createViewDefaults } from './views.js';

export const state = {
    // Current authenticated user (null = anonymous).
    me: null,
    projects: [],
    currentProject: null,
    tasks: [],
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
    editingTaskId: null,
    searchQuery: '',
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
    // View-specific state (kanban, table, timeline, calendar) — see views.js
    ...createViewDefaults(),
};
