// CO-393: Calendar lens — wraps the existing calendar/gantt view.
import { registerLens } from './registry.js';
import {
    renderCalendar, renderGantt, renderEventsTimeline, injectCalendarCallbacks,
} from '../../../a/modules/views/calendar.js';

export { injectCalendarCallbacks };

export const calendarLens = {
    id: 'calendar',
    label: 'Calendário',
    icon: 'calendar_month',
    // Calendar supports content types with a calendar date field in presentation.
    supports(contentType) {
        const pres = contentType?.presentation || {};
        const schema = contentType?.schema || {};
        return !!(pres.calendar?.date_field || schema.due_date || schema.date || schema.start_date);
    },
    render() { renderCalendar(); },
};

export const ganttLens = {
    id: 'gantt',
    label: 'Gantt',
    icon: 'view_timeline',
    supports(contentType) {
        const schema = contentType?.schema || {};
        return !!(schema.start_date || schema.due_date || schema.end_date);
    },
    render(viewDef) { renderGantt(viewDef); },
};

registerLens(calendarLens);
registerLens(ganttLens);
export { renderGantt, renderEventsTimeline };
