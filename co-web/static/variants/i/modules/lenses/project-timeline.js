// CO-396: project-timeline lens — roadmap/gantt of a project universe.
//
// The same entries the kanban board shows, rendered on a time axis: tasks in
// lanes (by epic/module/status), releases/milestones as markers, and a
// "no date" backlog so nothing disappears. Content × form: the quadro and the
// timeline are two forms of the same canonical entries — no new data model.
//
// An instance of the CO-393 lens frame (one-surface rule): registers via
// registerLens and renders through the registry dispatch, so it shows up as a
// header tab next to Kanban — the toggle quadro ⇄ timeline. The layout math is
// the shared engine (game_core::time_layout, mirrored in co-time.js); this lens
// only mounts the <co-time-grid> primitive in `roadmap` view mode.

import { registerLens } from './registry.js';
import { state } from '/variants/a/modules/state.js';
import '/shared/lib/co-time-grid.js';

export const projectTimelineLens = {
    id: 'project-timeline',
    label: (window.t && window.t('lens.project_timeline') !== 'lens.project_timeline')
        ? window.t('lens.project_timeline') : 'Roadmap',
    icon: 'timeline',
    namedViews: true,
    // Any content type with a status (board item) or a date field — i.e. the
    // same entries the kanban renders — can be laid out on the roadmap.
    supports(contentType) {
        const schema = contentType?.schema || {};
        const pres = contentType?.presentation || {};
        return !!(schema.status || schema.due_date || schema.start_date || schema.date
            || schema.event_at || pres.calendar?.date_field || pres.board);
    },
    render(viewDef) {
        const content = document.querySelector('#content');
        if (!content) return;
        content.className = 'content';
        content.replaceChildren();
        const el = document.createElement('co-time-grid');
        el.setAttribute('universe', state.currentUniverseSlug || 'template');
        el.setAttribute('view-mode', viewDef?.view_mode || 'roadmap');
        // Lane grouping: manifest view def → default 'status' (kanban-native).
        el.setAttribute('lane-by', viewDef?.lane_by || 'status');
        if (viewDef?.lens) el.setAttribute('lens', viewDef.lens);
        el.style.padding = '12px';
        content.appendChild(el);
    },
};

registerLens(projectTimelineLens);
