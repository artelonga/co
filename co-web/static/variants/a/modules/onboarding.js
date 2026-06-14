// ===== Onboarding + Criar Universo modal =====
import { state } from './state.js';
import { api, apiFetch } from './api.js';
import { esc } from './helpers.js';

let _render = () => {};
let _showLoginModal = () => {};
let _showToast = () => {};
let _setUniverseSlugInUrl = () => {};
let _bootAppForUniverse = async () => {};
let _hideTemplateBanner = () => {};
let _renderUsageCount = () => {};
let _loadMeUniverses = async () => {};

// CO-96: set inside setupCriarModal so other modules (sidebar context menu) can
// open the create/duplicate modal. `openCriarModal({copyFrom})` pre-fills the
// duplicate source.
let _openCriarModal = () => {};
export function openCriarModal(defaults) { _openCriarModal(defaults); }

export function injectOnboardingCallbacks(callbacks) {
    _render = callbacks.render;
    _showLoginModal = callbacks.showLoginModal;
    _showToast = callbacks.showToast;
    _setUniverseSlugInUrl = callbacks.setUniverseSlugInUrl;
    _bootAppForUniverse = callbacks.bootAppForUniverse;
    _hideTemplateBanner = callbacks.hideTemplateBanner;
    _renderUsageCount = callbacks.renderUsageCount;
    if (callbacks.loadMeUniverses) _loadMeUniverses = callbacks.loadMeUniverses;
}

export function setupOnboarding() {
    const banner = document.getElementById('onboarding-banner');
    if (!banner) return;

    if (/(?:^|;\s*)co_onboarded=1(?:;|$)/.test(document.cookie || '')) return;

    if (window.innerWidth < 720) return;

    if (!state.isTemplate) return;

    const titleEl = document.getElementById('onboarding-title');
    const bodyEl = document.getElementById('onboarding-body');
    const stepEl = document.getElementById('onboarding-step-indicator');
    const skipBtn = document.getElementById('onboarding-skip');
    const nextBtn = document.getElementById('onboarding-next');
    if (!titleEl || !bodyEl || !stepEl || !skipBtn || !nextBtn) return;

    const steps = [
        {
            title: 'Visões',
            bodyHtml: 'O Co tem várias visões do mesmo conteúdo: <strong>Quadro</strong>, <strong>Tabela</strong>, <strong>Conteúdo</strong>, <strong>Linha do tempo</strong>. Clique nas abas acima para alternar.',
            cta: 'Próximo',
        },
        {
            title: 'Linha do tempo',
            bodyHtml: 'A linha do tempo mostra eventos numa escala log: do Big Bang ao agora. <a href="/shared/timeline.html?u=tempo,universo,humanity" target="_blank" rel="noopener">Abrir linha do tempo →</a>',
            cta: 'Próximo',
        },
        {
            title: 'Crie seu universo',
            bodyHtml: 'Quando quiser organizar suas próprias coisas, faça login e clique em <strong>+ Novo universo</strong> na barra lateral.<br><a href="#login" class="onboarding-criar-link" style="color:var(--accent,#6366f1);text-decoration:underline;cursor:pointer">Criar conta</a>',
            cta: 'Concluir',
        },
    ];

    let current = 0;

    function renderStep() {
        const s = steps[current];
        titleEl.textContent = s.title;
        bodyEl.innerHTML = s.bodyHtml;
        stepEl.textContent = `${current + 1} / 3`;
        nextBtn.textContent = s.cta;
    }

    bodyEl.addEventListener('click', e => {
        const link = e.target.closest('.onboarding-criar-link');
        if (link) {
            e.preventDefault();
            _showLoginModal();
        }
    });

    function dismiss() {
        document.cookie = 'co_onboarded=1; Path=/; Max-Age=31536000; SameSite=Lax';
        banner.classList.add('hidden');
    }

    nextBtn.addEventListener('click', () => {
        if (current < steps.length - 1) {
            current += 1;
            renderStep();
        } else {
            dismiss();
        }
    });
    skipBtn.addEventListener('click', dismiss);

    renderStep();
    banner.classList.remove('hidden');
}

export function setupCriarModal() {
    const overlay = document.getElementById('criar-modal-overlay');
    if (!overlay) return;

    const closeBtn = document.getElementById('criar-modal-close');
    const cancelBtn = document.getElementById('criar-cancel');
    const form = document.getElementById('criar-form');
    const nameInput = document.getElementById('criar-name');
    const slugInput = document.getElementById('criar-slug');
    const descInput = document.getElementById('criar-description');
    const copyToggle = document.getElementById('criar-copy-from-toggle');
    const copySource = document.getElementById('criar-copy-from-source');
    const errorEl = document.getElementById('criar-error');

    // CO-450: provenance carried by the Yggdrasil "Criar no CO" deep-link
    // (?source=yggdrasil&instance=…). Stashed when the modal opens prefilled so
    // the universe_created telemetry event can measure YG→CO federation
    // conversion. Reset on every open() so a non-deep-link create stays clean.
    let _criarSource = null;
    let _criarInstance = null;

    function populateCopyFromSources() {
        if (!copySource) return;
        const universes = (state.userUniverses || [])
            .filter(u => u.key && u.key !== 'template');
        copySource.innerHTML = '<option value="template">Template (CO)</option>'
            + universes.map(u =>
                `<option value="${u.key}">${esc(u.name || u.key)}</option>`
            ).join('');
    }

    function open(defaults) {
        const d = defaults || {};
        // CO-450: `prefill` carries the deep-link `name`/`key` (and provenance).
        const prefill = d.prefill || {};
        _criarSource = prefill.source || null;
        _criarInstance = prefill.instance || null;
        populateCopyFromSources();
        overlay.classList.remove('hidden');
        nameInput.value = prefill.name || '';
        slugInput.value = prefill.key || '';
        if (descInput) descInput.value = '';
        const privacyRadio = form.querySelector('input[name="criar-visibility"][value="private"]');
        if (privacyRadio) privacyRadio.checked = true;
        // CO-96: when invoked as "Duplicate…" from the context menu, the source
        // is pre-filled and the copy-from toggle is forced on.
        const titleEl = document.getElementById('criar-modal-title');
        if (copyToggle) {
            const preCopy = d.copyFrom || (d.copyFromTemplate ? 'template' : null);
            copyToggle.checked = !!preCopy;
            if (copySource) {
                copySource.classList.toggle('hidden', !copyToggle.checked);
                if (preCopy) copySource.value = preCopy;
            }
        }
        if (titleEl) {
            titleEl.textContent = d.copyFrom ? 'Duplicar universo' : 'Criar universo';
        }
        if (errorEl) errorEl.classList.add('hidden');
        // CO-96: reset inline-validation state so a reopened modal starts clean.
        const sb = document.getElementById('criar-submit');
        if (sb) sb.disabled = false;
        // CO-450: when prefilled (deep-link), refresh the slug preview and run
        // the CO-96 inline validation so an invalid `key`/`name` surfaces its
        // error immediately instead of creating blindly on submit.
        updateSlugPreview();
        if (slugInput.value.trim()) validateKey();
        nameInput.focus();
    }

    function close() {
        overlay.classList.add('hidden');
    }

    if (copyToggle && copySource) {
        copyToggle.addEventListener('change', () => {
            copySource.classList.toggle('hidden', !copyToggle.checked);
        });
    }

    const slugPreview = document.getElementById('criar-slug-preview');
    const slugValEl = document.getElementById('criar-slug-val');
    const submitBtn = document.getElementById('criar-submit');

    function updateSlugPreview() {
        const slug = slugInput.value.trim();
        if (slugValEl) slugValEl.textContent = slug || '…';
        if (slugPreview) {
            if (slug) {
                slugPreview.setAttribute('data-active', '');
            } else {
                slugPreview.removeAttribute('data-active');
            }
        }
    }

    // CO-96: inline key validation. Format is checked synchronously; uniqueness
    // is checked against the API on a debounce so each keystroke doesn't fire a
    // request. `keyOk` gates the submit button.
    let keyOk = false;
    let keyCheckTimer = null;
    let keyCheckSeq = 0;

    function showKeyError(msg) {
        keyOk = false;
        if (errorEl) {
            errorEl.textContent = msg;
            errorEl.classList.remove('hidden');
        }
        if (submitBtn) submitBtn.disabled = true;
    }

    function clearKeyError() {
        if (errorEl) errorEl.classList.add('hidden');
        if (submitBtn) submitBtn.disabled = false;
    }

    function validateKeyFormat(key) {
        if (!key) return 'Informe um slug.';
        if (key.length < 2 || key.length > 40) return 'O slug precisa ter de 2 a 40 caracteres.';
        if (!/^[a-z0-9-]+$/.test(key)) return 'Use apenas letras minúsculas, números e hífens.';
        return null;
    }

    function validateKey() {
        const key = slugInput.value.trim();
        const formatErr = validateKeyFormat(key);
        if (formatErr) {
            showKeyError(formatErr);
            return;
        }
        // Format is fine; provisionally allow while we debounce the uniqueness check.
        keyOk = true;
        clearKeyError();
        if (keyCheckTimer) clearTimeout(keyCheckTimer);
        const seq = ++keyCheckSeq;
        keyCheckTimer = setTimeout(async () => {
            const available = await api.isUniverseKeyAvailable(key);
            // Ignore stale responses if the user kept typing.
            if (seq !== keyCheckSeq) return;
            if (slugInput.value.trim() !== key) return;
            if (!available) {
                showKeyError(`O slug "${key}" já está em uso. Escolha outro.`);
            } else {
                keyOk = true;
                clearKeyError();
            }
        }, 350);
    }

    nameInput.addEventListener('input', () => {
        const slug = nameInput.value
            .toLowerCase()
            .replace(/[^a-z0-9]+/g, '-')
            .replace(/^-+|-+$/g, '')
            .slice(0, 40);
        slugInput.value = slug;
        updateSlugPreview();
        validateKey();
    });

    slugInput.addEventListener('input', () => {
        updateSlugPreview();
        validateKey();
    });

    closeBtn && closeBtn.addEventListener('click', close);
    cancelBtn && cancelBtn.addEventListener('click', close);
    overlay.addEventListener('click', e => { if (e.target === overlay) close(); });

    form.addEventListener('submit', async e => {
        e.preventDefault();
        const name = nameInput.value.trim();
        const key = slugInput.value.trim();
        if (!name || !key) return;
        // CO-96: block submission on an invalid key format.
        const formatErr = validateKeyFormat(key);
        if (formatErr) {
            showKeyError(formatErr);
            return;
        }
        const description = descInput ? descInput.value.trim() : '';
        const visibilityEl = form.querySelector('input[name="criar-visibility"]:checked');
        const visibility = visibilityEl ? visibilityEl.value : 'private';
        const copyFrom = (copyToggle && copyToggle.checked && copySource)
            ? copySource.value
            : null;

        const submitBtn = document.getElementById('criar-submit');
        if (submitBtn) submitBtn.disabled = true;
        if (errorEl) errorEl.classList.add('hidden');

        let result;
        if (copyFrom) {
            // CO-96: `/clone` is anonymous-friendly but only works on public /
            // template sources; `/duplicate` (CO-95) copies ANY universe the
            // caller can read but requires auth. Use clone for the template
            // (so anonymous visitors keep working) and duplicate for everything
            // else (a user copying their own private universe).
            result = copyFrom === 'template'
                ? await api.cloneUniverse(copyFrom, { name, key, description })
                : await api.duplicateUniverse(copyFrom, { name, key, description });
        } else {
            result = await apiFetch('/api/v1/universes', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ key, name, description }),
            }, true);
            if (result && visibility !== 'private') {
                await apiFetch(`/api/v1/universes/${encodeURIComponent(key)}`, {
                    method: 'PUT',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ visibility }),
                }, true);
            }
        }
        if (submitBtn) submitBtn.disabled = false;

        if (!result) return;

        // CO-450: measure federation conversion. Emit on every create; carry
        // `source`/`instance` only when they came from the Yggdrasil deep-link.
        if (typeof window.coTrack === 'function') {
            const props = { copy_from: copyFrom ? 'yes' : 'no' };
            if (_criarSource) props.source = _criarSource;
            if (_criarInstance) props.instance = _criarInstance;
            window.coTrack('universe_created', props);
        }

        close();
        _showToast('Universo criado! Redirecionando...', 'success');
        _setUniverseSlugInUrl(result.key);
        state.currentUniverseSlug = result.key;
        state.isTemplate = false;
        state.universeInfo = {
            key: result.key,
            name: result.name,
            description: result.description,
            content_count: result.content_count || 0,
            is_anonymous: result.owner_id ? result.owner_id.startsWith('anon-') : true,
            is_template: false,
        };
        _renderUsageCount();
        _hideTemplateBanner();
        await _loadMeUniverses();
        await _bootAppForUniverse(result.key);
    });

    // CO-96: expose the modal opener so the sidebar context menu can launch the
    // "Duplicate…" flow with the source pre-filled.
    _openCriarModal = open;

    const btnSidebarNew = document.getElementById('btn-sidebar-new-universe');
    if (btnSidebarNew) btnSidebarNew.addEventListener('click', () => open());

    const btnCriar = document.getElementById('btn-criar-universo');
    if (btnCriar) btnCriar.addEventListener('click', () => open({ copyFromTemplate: true }));

    const btnEntrar = document.getElementById('btn-banner-entrar');
    // Same defer-dereference pattern as sidebar.js — the inject callback may
    // run after this binds; arrow wrapper reads `_showLoginModal` at click time.
    if (btnEntrar) btnEntrar.addEventListener('click', () => _showLoginModal());

    const btnBannerLang = document.getElementById('btn-banner-lang');
    if (btnBannerLang) {
        btnBannerLang.addEventListener('click', () => {
            window.setLang(window.currentLang === 'pt' ? 'en' : 'pt');
            _render();
        });
    }
}
