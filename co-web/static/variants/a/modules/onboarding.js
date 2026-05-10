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
            bodyHtml: 'Quando quiser organizar suas próprias coisas, faça login e clique em <strong>+ Novo universo</strong> na barra lateral.',
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
        populateCopyFromSources();
        overlay.classList.remove('hidden');
        nameInput.value = '';
        slugInput.value = '';
        if (descInput) descInput.value = '';
        const privacyRadio = form.querySelector('input[name="criar-visibility"][value="private"]');
        if (privacyRadio) privacyRadio.checked = true;
        if (copyToggle) {
            copyToggle.checked = !!d.copyFromTemplate;
            if (copySource) {
                copySource.classList.toggle('hidden', !copyToggle.checked);
                if (d.copyFromTemplate) copySource.value = 'template';
            }
        }
        if (errorEl) errorEl.classList.add('hidden');
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

    nameInput.addEventListener('input', () => {
        const slug = nameInput.value
            .toLowerCase()
            .replace(/[^a-z0-9]+/g, '-')
            .replace(/^-+|-+$/g, '')
            .slice(0, 40);
        slugInput.value = slug;
        updateSlugPreview();
    });

    slugInput.addEventListener('input', updateSlugPreview);

    closeBtn && closeBtn.addEventListener('click', close);
    cancelBtn && cancelBtn.addEventListener('click', close);
    overlay.addEventListener('click', e => { if (e.target === overlay) close(); });

    form.addEventListener('submit', async e => {
        e.preventDefault();
        const name = nameInput.value.trim();
        const key = slugInput.value.trim();
        if (!name || !key) return;
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
            result = await api.cloneUniverse(copyFrom, { name, key, description });
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
