// ===== Sidebar one-time event wiring (hamburger menu, project list toggle) =====
// CO-280 Phase 1: render the Platforms + Tools sections at boot so they're
// visible even before the first universe boot completes. The full renderSidebar
// also re-renders them, but doing it here avoids a flicker of empty sections
// while the universe nav is still loading.
import { renderPlatforms } from './platforms.js';
import { renderTools } from './tools.js';

export function setupHamburgerMenu() {
    renderPlatforms();
    renderTools();

    const hamburgerBtn = document.getElementById('hamburger-btn');
    const sidebar = document.getElementById('sidebar');
    const overlay = document.getElementById('sidebar-overlay');

    if (hamburgerBtn && sidebar) {
        hamburgerBtn.addEventListener('click', () => {
            sidebar.classList.toggle('open');
            if (overlay) overlay.classList.toggle('visible');
        });
    }

    if (overlay && sidebar) {
        overlay.addEventListener('click', () => {
            sidebar.classList.remove('open');
            overlay.classList.remove('visible');
        });
    }

    const toggleProjects = document.getElementById('btn-toggle-projects');
    const projectList = document.getElementById('project-list');
    if (toggleProjects && projectList) {
        toggleProjects.addEventListener('click', () => {
            const expanded = toggleProjects.getAttribute('aria-expanded') === 'true';
            toggleProjects.setAttribute('aria-expanded', String(!expanded));
            projectList.classList.toggle('collapsed', expanded);
        });
    }
}
