/**
 * Pointer-event drag primitive.
 *
 * Replaces HTML5 DnD with unified pointer events (mouse + touch + pen),
 * which fire correctly on iOS Safari and Android Chrome.
 *
 * Drag activates after a 200 ms hold OR 8 px of horizontal movement.
 * Purely vertical movement before the threshold is treated as scroll intent
 * and cancels the gesture, preserving native scroll behaviour on touch screens.
 *
 * Reusable across kanban board (CO-356) and Sala canvas (CO-352).
 */

const DRAG_THRESHOLD_PX = 8;  // horizontal movement in px
const DRAG_HOLD_MS = 200;     // hold duration in ms (iOS long-press convention)

/**
 * Attach drag detection to a container via event delegation.
 *
 * @param {HTMLElement} container      Element that owns draggable items.
 * @param {string}      cardSelector   CSS selector for draggable items inside container.
 * @param {object}      handlers
 * @param {(card: HTMLElement, pointerId: number, x: number, y: number) => void} handlers.onDragStart
 * @param {(x: number, y: number) => void} handlers.onDragMove
 * @param {(x: number, y: number) => void} handlers.onDragEnd
 * @param {() => void}  handlers.onDragCancel
 * @returns {() => void} Detach function — removes all listeners and cancels
 *   any in-progress gesture without calling onDragEnd/onDragCancel.
 */
export function attachPointerDrag(container, cardSelector, { onDragStart, onDragMove, onDragEnd, onDragCancel }) {
    // Reference to the cleanup function for the currently active gesture, so
    // re-renders (which call the returned detach fn) can cancel it safely.
    let activeDetach = null;

    function onPointerDown(e) {
        if (e.pointerType === 'mouse' && e.button !== 0) return;

        const card = e.target.closest(cardSelector);
        if (!card || !container.contains(card)) return;

        // Only one gesture at a time.
        if (activeDetach) activeDetach();

        const startX = e.clientX;
        const startY = e.clientY;
        let lastX = startX;
        let lastY = startY;
        let dragActive = false;

        // 200 ms hold → activate drag (iOS long-press convention for mobile).
        const holdTimer = setTimeout(() => {
            if (!dragActive) {
                dragActive = true;
                onDragStart(card, e.pointerId, lastX, lastY);
            }
        }, DRAG_HOLD_MS);

        function handleMove(ev) {
            if (ev.pointerId !== e.pointerId) return;
            lastX = ev.clientX;
            lastY = ev.clientY;

            if (!dragActive) {
                const dx = Math.abs(ev.clientX - startX);
                const dy = Math.abs(ev.clientY - startY);

                if (dx >= DRAG_THRESHOLD_PX) {
                    // Horizontal movement threshold exceeded → start drag.
                    clearTimeout(holdTimer);
                    dragActive = true;
                    // Pointer capture routes subsequent events to the card even
                    // when the pointer leaves it (important for fast mouse moves).
                    try { card.setPointerCapture(ev.pointerId); } catch (_) {}
                    onDragStart(card, ev.pointerId, ev.clientX, ev.clientY);
                } else if (dy > DRAG_THRESHOLD_PX && dy > dx) {
                    // Vertical-first movement: treat as scroll, abort gesture.
                    clearTimeout(holdTimer);
                    detach();
                    return;
                }
            }

            if (dragActive) {
                // Prevent touch scroll while dragging.
                ev.preventDefault();
                onDragMove(ev.clientX, ev.clientY);
            }
        }

        function handleUp(ev) {
            if (ev.pointerId !== e.pointerId) return;
            const wasDragging = dragActive;
            detach();
            if (wasDragging) onDragEnd(ev.clientX, ev.clientY);
        }

        function handleCancel(ev) {
            if (ev.pointerId !== e.pointerId) return;
            const wasDragging = dragActive;
            detach();
            if (wasDragging) onDragCancel();
        }

        function handleKey(ev) {
            if (ev.key === 'Escape' && dragActive) {
                detach();
                onDragCancel();
            }
        }

        function detach() {
            clearTimeout(holdTimer);
            dragActive = false;
            document.removeEventListener('pointermove', handleMove);
            document.removeEventListener('pointerup', handleUp);
            document.removeEventListener('pointercancel', handleCancel);
            document.removeEventListener('keydown', handleKey);
            if (activeDetach === detach) activeDetach = null;
        }

        activeDetach = detach;

        // { passive: false } required so we can call preventDefault on pointermove
        // to block touch scroll once a drag is active.
        document.addEventListener('pointermove', handleMove, { passive: false });
        document.addEventListener('pointerup', handleUp);
        document.addEventListener('pointercancel', handleCancel);
        document.addEventListener('keydown', handleKey);
    }

    container.addEventListener('pointerdown', onPointerDown);

    return function detachAll() {
        if (activeDetach) {
            activeDetach();
            activeDetach = null;
        }
        container.removeEventListener('pointerdown', onPointerDown);
    };
}
