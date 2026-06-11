// CO-393: Table lens — wraps the existing table view.
import { registerLens } from './registry.js';
import { renderTable, closeStatusDropdown, injectTableCallbacks } from '/variants/a/modules/views/table.js';

export { injectTableCallbacks };

export const tableLens = {
    id: 'table',
    label: 'Tabela',
    icon: 'table_rows',
    // Table supports all content types.
    supports(_contentType) { return true; },
    render() { renderTable(); },
};

registerLens(tableLens);
export { closeStatusDropdown };
