// ===== Universe-related state helpers =====
import { state } from './shape.js';

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
