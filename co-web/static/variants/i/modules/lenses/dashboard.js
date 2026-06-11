// CO-393: Dashboard lens — wraps the existing dashboard view.
import { registerLens } from './registry.js';
import { renderDashboard, injectDashboardCallbacks } from '../../../a/modules/views/dashboard.js';

export { injectDashboardCallbacks };

export const dashboardLens = {
    id: 'dashboard',
    label: 'Painel',
    icon: 'dashboard',
    // Dashboard works for any content type with trackable status/priority.
    supports(contentType) {
        const schema = contentType?.schema || {};
        return !!(schema.status || schema.priority);
    },
    render() { renderDashboard(); },
};

registerLens(dashboardLens);
