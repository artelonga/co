// CO-393: Kanban lens — wraps the existing kanban view.
import { registerLens } from './registry.js';
import { renderKanban, injectKanbanCallbacks } from '../../../a/modules/views/kanban.js';

export { injectKanbanCallbacks };

export const kanbanLens = {
    id: 'kanban',
    label: 'Kanban',
    icon: 'view_kanban',
    // Kanban supports content types that have a status field in their schema.
    supports(contentType) {
        const schema = contentType?.schema || {};
        return !!(schema.status || schema.Status);
    },
    render() { renderKanban(); },
};

registerLens(kanbanLens);
