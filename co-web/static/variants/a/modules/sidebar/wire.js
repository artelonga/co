// ===== Sidebar one-time event wiring (hamburger menu, project list toggle) =====
// CO-311: Platforms section removed; Tools renders early to avoid flicker.
// CO-358: swipe-from-left-edge, ESC close, focus trap added for drawer.
import { renderTools } from './tools.js';

export function setupHamburgerMenu() {
    renderTools();

    const hamburgerBtn = document.getElementById('hamburger-btn');
    const sidebar = document.getElementById('sidebar');
    const overlay = document.getElementById('sidebar-overlay');

    function openDrawer() {
        sidebar.classList.add('open');
        if (overlay) overlay.classList.add('visible');
        trapFocus(sidebar);
    }

    function closeDrawer() {
        sidebar.classList.remove('open');
        if (overlay) overlay.classList.remove('visible');
        releaseFocus();
        hamburgerBtn?.focus();
    }

    if (hamburgerBtn && sidebar) {
        hamburgerBtn.addEventListener('click', () => {
            if (sidebar.classList.contains('open')) closeDrawer();
            else openDrawer();
        });
    }

    if (overlay && sidebar) {
        overlay.addEventListener('click', closeDrawer);
    }

    // ESC closes the drawer
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape' && sidebar?.classList.contains('open')) {
            closeDrawer();
        }
    });

    // Swipe from left edge (start-x < 20px, deltaX > 60px) opens drawer.
    // Swipe right on open drawer closes it.
    let touchStartX = 0;
    let touchStartY = 0;
    document.addEventListener('touchstart', (e) => {
        touchStartX = e.touches[0].clientX;
        touchStartY = e.touches[0].clientY;
    }, { passive: true });

    document.addEventListener('touchend', (e) => {
        const dx = e.changedTouches[0].clientX - touchStartX;
        const dy = e.changedTouches[0].clientY - touchStartY;
        // Only horizontal swipes (dx dominates dy by 2:1)
        if (Math.abs(dx) < Math.abs(dy) * 2) return;
        if (!sidebar.classList.contains('open') && touchStartX < 20 && dx > 60) {
            openDrawer();
        } else if (sidebar.classList.contains('open') && dx < -60) {
            closeDrawer();
        }
    }, { passive: true });

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

// ===== Focus trap helpers =====
const FOCUSABLE = 'a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])';
let _trapCleanup = null;

function trapFocus(el) {
    releaseFocus();
    const focusable = () => Array.from(el.querySelectorAll(FOCUSABLE)).filter(n => !n.closest('[hidden]'));
    const handler = (e) => {
        if (e.key !== 'Tab') return;
        const items = focusable();
        if (!items.length) { e.preventDefault(); return; }
        const first = items[0];
        const last = items[items.length - 1];
        if (e.shiftKey) {
            if (document.activeElement === first) { e.preventDefault(); last.focus(); }
        } else {
            if (document.activeElement === last) { e.preventDefault(); first.focus(); }
        }
    };
    document.addEventListener('keydown', handler);
    _trapCleanup = () => document.removeEventListener('keydown', handler);
    // Move focus into drawer
    const first = focusable()[0];
    if (first) first.focus();
}

function releaseFocus() {
    if (_trapCleanup) { _trapCleanup(); _trapCleanup = null; }
}
