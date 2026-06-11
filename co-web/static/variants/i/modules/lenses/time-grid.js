// CO-387: time-grid lens — wraps the <co-time-grid> time-rendering primitive.
//
// An instance of the CO-393 lens frame (one-surface rule): registers via
// registerLens and renders through the registry dispatch in renderContent —
// no if/else chain edits. Supports named manifest views (namedViews), e.g.:
//
//   views:
//     - type: time-grid
//       name: 4day
//       label: "4-day week"
//       lens: 4-day-week
//       view_mode: grid
//
// The component fetches the universe's _calendar.yaml lenses + entries itself.

import { registerLens } from './registry.js';
import { state } from '/variants/a/modules/state.js';
import '/shared/lib/co-time-grid.js';

export const timeGridLens = {
    id: 'time-grid',
    label: (window.t && window.t('lens.time_grid') !== 'lens.time_grid')
        ? window.t('lens.time_grid') : 'Tempo',
    icon: 'calendar_view_week',
    namedViews: true,
    // Same eligibility as the calendar/timeline lenses: any content type with
    // a date field can be rendered on a time axis.
    supports(contentType) {
        const pres = contentType?.presentation || {};
        const schema = contentType?.schema || {};
        return !!(pres.calendar?.date_field || schema.due_date || schema.date
            || schema.start_date || schema.event_at);
    },
    render(viewDef) {
        const content = document.querySelector('#content');
        if (!content) return;
        content.className = 'content';
        content.innerHTML = '';
        const el = document.createElement('co-time-grid');
        el.setAttribute('universe', state.currentUniverseSlug || 'template');
        if (viewDef?.lens) el.setAttribute('lens', viewDef.lens);
        if (viewDef?.view_mode) el.setAttribute('view-mode', viewDef.view_mode);
        el.style.padding = '12px';
        content.appendChild(el);
    },
};

registerLens(timeGridLens);
