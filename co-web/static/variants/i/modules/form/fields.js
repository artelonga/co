// CO-393: Form field renderers — one renderer per schema field type.
// All renderers return an HTMLElement with a name attribute matching the field key.

import { esc } from '../../../a/modules/helpers.js';

const t = (k) => (window.t ? window.t(k) : k);

// Renders a single form field element based on field definition.
// fieldKey: the schema key (e.g. 'title', 'status')
// def: { type, required, label, values, placeholder }
// value: current value (optional)
export function renderField(fieldKey, def, value) {
    const label = def.label || fieldKey;
    const required = def.required ? ` <span class="form-required" aria-label="${t('form.required')}" title="${t('form.required')}">*</span>` : '';
    const wrapper = document.createElement('div');
    wrapper.className = 'form-group form-field';
    wrapper.dataset.fieldKey = fieldKey;

    let inputHtml = '';
    const v = value !== undefined && value !== null ? value : (def.default !== undefined ? def.default : '');
    const id = `field-${esc(fieldKey)}`;
    const ph = esc(def.placeholder || '');

    switch (def.type) {
        case 'text':
        case 'markdown':
            inputHtml = `<textarea class="form-control" name="${esc(fieldKey)}" id="${id}" rows="4" ${def.required ? 'required' : ''} placeholder="${ph}">${esc(String(v))}</textarea>`;
            break;
        case 'number':
            inputHtml = `<input class="form-control" type="number" name="${esc(fieldKey)}" id="${id}" value="${esc(String(v))}" ${def.required ? 'required' : ''} placeholder="${ph}">`;
            break;
        case 'date':
            inputHtml = `<input class="form-control" type="date" name="${esc(fieldKey)}" id="${id}" value="${esc(String(v))}" ${def.required ? 'required' : ''}>`;
            break;
        case 'boolean':
            inputHtml = `<label class="form-check-label"><input type="checkbox" name="${esc(fieldKey)}" id="${id}" ${v ? 'checked' : ''}> ${esc(label)}</label>`;
            break;
        case 'enum': {
            const opts = (def.values || []).map(val =>
                `<option value="${esc(String(val))}" ${String(v) === String(val) ? 'selected' : ''}>${esc(String(val))}</option>`
            ).join('');
            inputHtml = `<select class="form-control" name="${esc(fieldKey)}" id="${id}" ${def.required ? 'required' : ''}><option value=""></option>${opts}</select>`;
            break;
        }
        case 'ref': {
            // Cross-universe entry picker: text input + data attribute for progressive enhancement
            inputHtml = `<input class="form-control form-field-ref" type="text" name="${esc(fieldKey)}" id="${id}" value="${esc(String(v))}" placeholder="${esc(def.placeholder || t('form.field_ref'))}" data-ref-universe="${esc(def.universe || '')}" ${def.required ? 'required' : ''}>`;
            break;
        }
        case 'string':
        default:
            inputHtml = `<input class="form-control" type="text" name="${esc(fieldKey)}" id="${id}" value="${esc(String(v))}" ${def.required ? 'required' : ''} placeholder="${ph}">`;
            break;
    }

    if (def.type === 'boolean') {
        wrapper.innerHTML = `<div class="form-check">${inputHtml}</div>`;
    } else {
        wrapper.innerHTML = `<label for="${id}" class="form-label">${esc(label)}${required}</label>${inputHtml}`;
    }
    return wrapper;
}

// Collect the value of a single field element.
export function collectField(fieldKey, def, fieldEl) {
    if (!fieldEl) return undefined;
    if (def.type === 'boolean') {
        const cb = fieldEl.querySelector(`[name="${fieldKey}"]`);
        return cb ? cb.checked : false;
    }
    if (def.type === 'number') {
        const inp = fieldEl.querySelector(`[name="${fieldKey}"]`);
        if (!inp) return undefined;
        const n = Number(inp.value);
        return isNaN(n) ? undefined : n;
    }
    const el = fieldEl.querySelector(`[name="${fieldKey}"]`);
    if (!el) return undefined;
    return el.value.trim() || undefined;
}
