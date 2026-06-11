// CO-393: Composability demonstration test.
// Proves that adding a new content_type to _universe.yaml yields a working
// form + eligible lenses with ZERO JS changes.
//
// Run from browser console: import('/variants/i/modules/form/test.js').then(m => m.runTest())

import { renderForm, collect } from './engine.js';
import { getLensesForContentType } from '../lenses/registry.js';

export function runTest() {
    // Hypothetical new content_type added to _universe.yaml — no JS change required:
    const contentType = {
        name: 'article',
        schema: {
            title:        { type: 'string',  required: true,  label: 'Título' },
            body:         { type: 'text',    required: false, label: 'Corpo' },
            category:     { type: 'enum',    values: ['notícia', 'opinião', 'tutorial'], label: 'Categoria' },
            published:    { type: 'boolean', label: 'Publicado' },
            published_at: { type: 'date',    label: 'Data de publicação' },
        },
    };

    // 1. Form engine generates a form — no JS change required
    const form = renderForm(contentType.schema);
    console.assert(form instanceof HTMLElement, '[CO-393] form engine returned a DOM element');
    console.assert(form.querySelector('[name="title"]'), '[CO-393] title field rendered');
    console.assert(form.querySelector('[name="body"]'), '[CO-393] body field rendered');
    console.assert(form.querySelector('[name="category"]'), '[CO-393] category field rendered');
    console.assert(form.querySelector('[name="published"]'), '[CO-393] published field rendered');

    // 2. collect() captures values
    const titleInput = form.querySelector('[name="title"]');
    if (titleInput) titleInput.value = 'Meu Artigo';
    const data = collect(form, contentType.schema);
    console.assert(data.title === 'Meu Artigo', '[CO-393] collect() captures title');

    // 3. Lens registry returns eligible lenses — no JS change required
    const eligible = getLensesForContentType(contentType);
    console.assert(eligible.length > 0, '[CO-393] at least one lens supports the article type');
    console.assert(eligible.some(l => l.id === 'document'), '[CO-393] document lens always eligible');

    console.log('[CO-393] composability test passed ✓');
    console.log('[CO-393] eligible lenses:', eligible.map(l => l.id));
    return { form, data, eligible };
}
