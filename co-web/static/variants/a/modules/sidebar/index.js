// ===== sidebar/ — public re-exports =====
// Preserves every import that used `from './sidebar.js'` before this split.
export { renderSidebar, injectSidebarCallbacks, injectSetUniverseSlugInUrl } from './render.js';
export { renderHeader, renderUsageCount, incrementLocalUsageCount, renderHeaderUserArea, injectShowLoginModal } from './header.js';
export { renderUserBadge } from './badge.js';
export { renderMiniCalendar, injectScrollToDate } from './mini-calendar.js';
export { setupHamburgerMenu } from './wire.js';
// CO-280 Phase 1: three-section sidebar (Platforms / This universe / Tools)
export { renderPlatforms, PLATFORMS } from './platforms.js';
export { renderTools } from './tools.js';
