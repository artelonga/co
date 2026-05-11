// CO-202: Notification preferences settings section
import { apiFetch } from './api.js';
import { subscribeToPush, pushPermissionState, unsubscribeFromPush } from './push.js';

const EVENT_TYPES = [
    { key: 'chat_message', label: 'notif.event.chat_message' },
    { key: 'chat_dm',      label: 'notif.event.chat_dm' },
    { key: 'invitation',   label: 'notif.event.invitation' },
    { key: 'mention',      label: 'notif.event.mention' },
];

const CHANNELS = [
    { key: 'in_app', label: 'notif.settings.in_app' },
    { key: 'email',  label: 'notif.settings.email' },
    { key: 'push',   label: 'notif.settings.push' },
];

const FREQ_OPTIONS = [
    { key: 'instant', label: 'notif.email_freq.instant' },
    { key: 'hourly',  label: 'notif.email_freq.hourly' },
    { key: 'daily',   label: 'notif.email_freq.daily' },
    { key: 'weekly',  label: 'notif.email_freq.weekly' },
    { key: 'never',   label: 'notif.email_freq.never' },
];

function t(key) {
    return (window.t ? window.t(key) : null) || key;
}

async function savePrefs(patch) {
    await apiFetch('/api/v1/me/notification-preferences', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        credentials: 'include',
        body: JSON.stringify(patch),
    }, true);
}

export async function renderNotificationSettings(container) {
    if (!container) return;
    container.innerHTML = `<p class="form-hint">${t('loading')}</p>`;

    const prefs = await apiFetch('/api/v1/me/notification-preferences', {}, true);
    if (!prefs) { container.innerHTML = ''; return; }

    const matrixHeaders = CHANNELS.map(c => `<th>${t(c.label)}</th>`).join('');
    const matrixRows = EVENT_TYPES.map(ev =>
        `<tr><td class="notif-event-label">${t(ev.label)}</td>${CHANNELS.map(ch => {
            const key = `${ch.key}_${ev.key}`;
            return `<td><input type="checkbox" class="notif-matrix-cb" data-key="${key}" ${prefs[key] ? 'checked' : ''}></td>`;
        }).join('')}</tr>`
    ).join('');

    const freqRadios = FREQ_OPTIONS.map(f =>
        `<label class="form-radio"><input type="radio" name="notif_email_freq" value="${f.key}" ${prefs.email_digest_freq === f.key ? 'checked' : ''}> <span>${t(f.label)}</span></label>`
    ).join('');

    container.innerHTML = `
        <div class="notif-settings-matrix-wrap">
            <p class="form-hint notif-when-label">${t('notif.settings.when_label')}</p>
            <table class="notif-matrix">
                <thead><tr><th></th>${matrixHeaders}</tr></thead>
                <tbody>${matrixRows}</tbody>
            </table>
        </div>
        <div class="notif-freq-section">
            <p class="form-hint">${t('notif.email_freq.label')}</p>
            <div class="notif-freq-radios">${freqRadios}</div>
        </div>
        <div class="notif-quiet-section">
            <p class="form-hint">${t('notif.quiet.label')}</p>
            <div class="notif-quiet-row">
                <span>${t('notif.quiet.from')}</span>
                <input type="time" id="notif-quiet-start" value="${prefs.quiet_hours_start || ''}" class="notif-time-input">
                <span>${t('notif.quiet.to')}</span>
                <input type="time" id="notif-quiet-end" value="${prefs.quiet_hours_end || ''}" class="notif-time-input">
            </div>
            <div class="notif-quiet-tz-row">
                <span>${t('notif.quiet.tz')}:</span>
                <input type="text" id="notif-quiet-tz" value="${prefs.quiet_hours_tz || 'America/Sao_Paulo'}" class="notif-tz-input" placeholder="America/Sao_Paulo">
            </div>
        </div>
        <div id="notif-push-section" class="notif-push-section">
            <div id="notif-devices-list"></div>
        </div>
        <p id="notif-settings-msg" class="form-hint" style="min-height:1.2em"></p>`;

    _bindMatrix(container);
    _bindFreq(container);
    _bindQuiet(container);
    await _bindPush(container);
}

function _showMsg(container, msg) {
    const el = container.querySelector('#notif-settings-msg');
    if (!el) return;
    el.textContent = msg;
    setTimeout(() => { el.textContent = ''; }, 3000);
}

function _bindMatrix(container) {
    container.querySelectorAll('.notif-matrix-cb').forEach(cb => {
        cb.addEventListener('change', async () => {
            await savePrefs({ [cb.dataset.key]: cb.checked });
            _showMsg(container, t('notif.saved'));
        });
    });
}

function _bindFreq(container) {
    container.querySelectorAll('input[name="notif_email_freq"]').forEach(radio => {
        radio.addEventListener('change', async () => {
            if (!radio.checked) return;
            await savePrefs({ email_digest_freq: radio.value });
            _showMsg(container, t('notif.saved'));
        });
    });
}

function _bindQuiet(container) {
    let timer = null;
    const save = async () => {
        const start = container.querySelector('#notif-quiet-start')?.value || null;
        const end = container.querySelector('#notif-quiet-end')?.value || null;
        const tz = container.querySelector('#notif-quiet-tz')?.value || 'America/Sao_Paulo';
        await savePrefs({ quiet_hours_start: start || null, quiet_hours_end: end || null, quiet_hours_tz: tz });
        _showMsg(container, t('notif.saved'));
    };
    ['#notif-quiet-start', '#notif-quiet-end', '#notif-quiet-tz'].forEach(sel => {
        container.querySelector(sel)?.addEventListener('change', () => {
            clearTimeout(timer);
            timer = setTimeout(save, 500);
        });
    });
}

async function _bindPush(container) {
    const section = container.querySelector('#notif-push-section');
    if (!section) return;

    const pstate = pushPermissionState();
    if (pstate === 'unsupported') {
        const msg = document.createElement('p');
        msg.className = 'form-hint';
        msg.textContent = t('notif.push.not_supported');
        section.prepend(msg);
        return;
    }

    if (pstate !== 'granted') {
        const btn = document.createElement('button');
        btn.id = 'btn-push-subscribe';
        btn.className = 'btn btn-secondary btn-sm';
        btn.textContent = t('notif.push.activate');
        section.prepend(btn);
        btn.addEventListener('click', async () => {
            btn.disabled = true;
            const sub = await subscribeToPush();
            if (sub) {
                btn.remove();
                await _renderDevices(container);
                _showMsg(container, t('notif.saved'));
            } else {
                btn.disabled = false;
            }
        });
    }

    await _renderDevices(container);
}

async function _renderDevices(container) {
    const devicesList = container.querySelector('#notif-devices-list');
    if (!devicesList) return;
    devicesList.innerHTML = '';

    const data = await apiFetch('/api/v1/me/push-subscriptions', {}, true);
    if (!data || !data.subscriptions || data.subscriptions.length === 0) return;

    const label = document.createElement('p');
    label.className = 'form-hint';
    label.textContent = t('notif.push.devices');
    devicesList.appendChild(label);

    data.subscriptions.forEach(sub => {
        const row = document.createElement('div');
        row.className = 'notif-device-row';
        const created = sub.created_at ? new Date(sub.created_at).toLocaleDateString() : '';
        row.innerHTML = `<span class="notif-device-name">• ${(sub.user_agent || '').slice(0, 60)}</span> — <span class="notif-device-date">${created}</span> `;
        const removeBtn = document.createElement('button');
        removeBtn.className = 'btn-text-sm';
        removeBtn.textContent = t('notif.push.remove');
        removeBtn.addEventListener('click', async () => {
            await unsubscribeFromPush(sub.id);
            await _renderDevices(container);
        });
        row.appendChild(removeBtn);
        devicesList.appendChild(row);
    });
}
