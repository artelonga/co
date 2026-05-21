// ===== User badge (sidebar user row) =====
import { renderHeaderUserArea } from './header.js';

export function renderUserBadge(me) {
    const sidebarUser = document.getElementById('sidebar-user');
    const nameEl = document.getElementById('user-display-name');
    if (sidebarUser) sidebarUser.classList.remove('hidden');
    if (nameEl) nameEl.textContent = me.display_name || me.email;
    renderHeaderUserArea(me);
}
