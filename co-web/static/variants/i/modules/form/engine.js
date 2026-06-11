// CO-393: Schema-driven form engine.
// renderForm(schema, value?) → HTMLFormElement
// collect(form) → value object
//
// schema: { [fieldKey]: { type, required, label, values, placeholder, default } }
// Matches the content_types[].schema format from _universe.yaml manifests.

import { renderField, collectField } from './fields.js';

// Renders a <form> from a schema definition and optional initial value.
// Returns the form element; caller appends it and calls collect() on submit.
export function renderForm(schema, value) {
    if (!schema || typeof schema !== 'object') {
        const form = document.createElement('form');
        form.className = 'co-form co-form-empty';
        form.innerHTML = '<p class="form-empty-hint">Sem campos definidos para este tipo.</p>';
        return form;
    }

    const form = document.createElement('form');
    form.className = 'co-form';
    form.noValidate = false;

    const keys = Object.keys(schema);
    for (const key of keys) {
        const def = schema[key] || {};
        const fieldValue = value && value[key] !== undefined ? value[key] : undefined;
        const fieldEl = renderField(key, def, fieldValue);
        form.appendChild(fieldEl);
    }

    return form;
}

// Collects values from a rendered form.
// Returns an object whose keys match the schema.
export function collect(form, schema) {
    if (!form || !schema) return {};
    const result = {};
    for (const key of Object.keys(schema)) {
        const def = schema[key] || {};
        const fieldEl = form.querySelector(`.form-field[data-field-key="${key}"]`);
        const val = collectField(key, def, fieldEl);
        if (val !== undefined) result[key] = val;
    }
    return result;
}

// Convenience: validate required fields. Returns array of error messages.
export function validate(form, schema) {
    const errors = [];
    if (!form || !schema) return errors;
    for (const [key, def] of Object.entries(schema)) {
        if (!def.required) continue;
        const fieldEl = form.querySelector(`.form-field[data-field-key="${key}"]`);
        const val = collectField(key, def, fieldEl);
        if (!val && val !== 0 && val !== false) {
            errors.push(`${def.label || key} é obrigatório`);
        }
    }
    return errors;
}

// Renders a universe create/edit form with standard fields.
// Returns { form, collect } where collect() → { name, key, description, visibility }.
export function renderUniverseForm(value) {
    const t = (k) => (window.t ? window.t(k) : k);
    const schema = {
        name: { type: 'string', required: true, label: t('form.name') },
        key: {
            type: 'string', required: true,
            label: t('form.slug'),
            placeholder: t('form.slug_hint'),
        },
        description: { type: 'text', required: false, label: 'Descrição' },
        visibility: {
            type: 'enum', required: false,
            label: t('criar.visibility'),
            values: ['private', 'public-subscribable'],
        },
    };
    const form = renderForm(schema, value);
    return {
        form,
        collect: () => collect(form, schema),
        validate: () => validate(form, schema),
    };
}
