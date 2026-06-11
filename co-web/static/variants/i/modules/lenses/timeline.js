// CO-393: Timeline lens — wraps the existing timeline view.
import { registerLens } from './registry.js';
import {
    renderTimeline, getTimelineRange, initTimelineStart, scrollToDate,
    injectTimelineCallbacks,
} from '../../../a/modules/views/timeline.js';

export { injectTimelineCallbacks, initTimelineStart, scrollToDate };

export const timelineLens = {
    id: 'timeline',
    label: 'Timeline',
    icon: 'timeline',
    // Timeline supports content types with a date field.
    supports(contentType) {
        const schema = contentType?.schema || {};
        const pres = contentType?.presentation || {};
        return !!(pres.calendar?.date_field || schema.due_date || schema.start_date || schema.date);
    },
    render() { renderTimeline(); },
};

registerLens(timelineLens);
export { getTimelineRange };
