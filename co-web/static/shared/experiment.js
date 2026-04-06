(function () {
    'use strict';

    // Theme tier data fetched from /api/v1/themes/available at init.
    // Defaults to the free tier so UI is correct before the fetch completes.
    let availableThemes = {
        palettes: ['scholarly', 'scholarly-dark', 'relic', 'relic-light'],
        variants: [],
        custom: null,
    };

    const VARIANTS = [
        { key: 'a', name: 'Modern' },
        { key: 'b', name: 'Medieval' },
        { key: 'c', name: 'Retro Arcade' },
        { key: 'd', name: 'Steampunk' },
        { key: 'e', name: 'Matrix' },
        { key: 'f', name: 'Cyberpunk' },
        { key: 'g', name: 'Garden' },
        { key: 'h', name: 'Terminal' },
    ];

    const VARIANT_COLORS = {
        a: { bg: '#f0f2f5', accent: '#6366f1' },
        b: { bg: '#F5E6D3', accent: '#8B4513' },
        c: { bg: '#1c1c1c', accent: '#d4a24c' },
        d: { bg: '#18191b', accent: '#7b8fa0' },
        e: { bg: '#000000', accent: '#00ff41' },
        f: { bg: '#0d0221', accent: '#ff2a6d' },
        g: { bg: '#f0f5e8', accent: '#4caf50' },
        h: { bg: '#000000', accent: '#ffffff' },
    };

    const PALETTE_KEYS = [
        { key: '--bg', label: 'Background' },
        { key: '--sidebar-bg', label: 'Sidebar' },
        { key: '--accent', label: 'Accent' },
        { key: '--text-primary', label: 'Text' },
        { key: '--card-bg', label: 'Cards' },
    ];

    const NAMED_PALETTES = [
        { key: '',              name: 'Modern',            bg: '#f0f2f5', accent: '#6366f1' },
        { key: 'scholarly',     name: 'Scholarly · Light', bg: '#FFF9ED', accent: '#CD7F32' },
        { key: 'scholarly-dark',name: 'Scholarly · Dark',  bg: '#1c1610', accent: '#CD7F32' },
        { key: 'relic',         name: 'Relic · Dark',      bg: '#131313', accent: '#e0505f' },
        { key: 'relic-light',   name: 'Relic · Light',     bg: '#F5F0F0', accent: '#af2b3e' },
    ];

    let currentNamedPalette = '';

    function loadNamedPalette() {
        const saved = localStorage.getItem('co_named_palette') || '';
        currentNamedPalette = saved;
        document.documentElement.setAttribute('data-palette', saved);
    }

    function applyNamedPalette(key) {
        currentNamedPalette = key;
        localStorage.setItem('co_named_palette', key);
        document.documentElement.setAttribute('data-palette', key);
        renderHeaderSwitcher();
    }

    let currentVariant = 'a';

    // --- Palette ---
    function loadPalette() {
        const saved = localStorage.getItem('co_custom_palette');
        if (saved) {
            try {
                const palette = JSON.parse(saved);
                Object.entries(palette).forEach(([k, v]) => {
                    document.documentElement.style.setProperty(k, v);
                });
            } catch (_) { /* ignore */ }
        }
    }

    function savePalette(palette) {
        localStorage.setItem('co_custom_palette', JSON.stringify(palette));
    }

    function getCurrentPalette() {
        const palette = {};
        const saved = localStorage.getItem('co_custom_palette');
        if (saved) {
            try { return JSON.parse(saved); } catch (_) { /* ignore */ }
        }
        return palette;
    }

    function getComputedPaletteValue(key) {
        const saved = getCurrentPalette();
        if (saved[key]) return saved[key];
        return getComputedStyle(document.documentElement).getPropertyValue(key).trim();
    }

    function resetPalette() {
        localStorage.removeItem('co_custom_palette');
        PALETTE_KEYS.forEach(({ key }) => {
            document.documentElement.style.removeProperty(key);
        });
    }

    // --- Variant switch (swap CSS, no full reload) ---
    async function applyVariantSwitch(variant) {
        try {
            await fetch('/api/experiment/variant', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ variant }),
            });
            resetPalette();
            currentVariant = variant;
            // Swap the stylesheet immediately so the new theme is applied without a page reload
            const styleLink = document.querySelector('link[href="/style.css"], link[href^="/style.css?"]');
            if (styleLink) {
                styleLink.href = '/style.css?' + Date.now();
            }
            // Update header switcher UI to reflect new selection
            renderHeaderSwitcher();
        } catch (_err) {
            showToast('Failed to switch variant');
        }
    }

    // --- Header Palette Switcher ---
    function swatchHtml(variantKey, dotClass) {
        const colors = VARIANT_COLORS[variantKey];
        if (!colors) return '';
        return `<span class="${dotClass}" style="background:${colors.bg}"></span><span class="${dotClass}" style="background:${colors.accent}"></span>`;
    }

    function variantLabel(key) {
        const v = VARIANTS.find(v => v.key === key);
        return v ? v.name : key.toUpperCase();
    }

    function renderHeaderSwitcher() {
        const slot = document.getElementById('palette-switcher');
        if (!slot) return;

        slot.innerHTML = '';

        // Filter palettes to only those available to this user's tier.
        const allowedPalettes = NAMED_PALETTES.filter(p =>
            availableThemes.palettes.includes(p.key)
        );

        // If the currently selected palette is not in the allowed list, fall back to the
        // first allowed one (this happens when a premium theme is displayed to a visitor —
        // the switcher just won't pre-select it, the CSS token on the page still applies).
        const current =
            allowedPalettes.find(p => p.key === currentNamedPalette) ||
            allowedPalettes[0] ||
            NAMED_PALETTES[0];

        const btn = document.createElement('button');
        btn.className = 'palette-switcher-btn';
        btn.id = 'palette-switcher-toggle';
        btn.setAttribute('aria-haspopup', 'listbox');
        btn.innerHTML = `
            <span class="palette-switcher-swatch">
                <span class="palette-switcher-swatch-dot" style="background:${current.bg}"></span>
                <span class="palette-switcher-swatch-dot" style="background:${current.accent}"></span>
            </span>
            <span class="palette-switcher-label">${esc(current.name)}</span>
            <svg class="palette-switcher-chevron" width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true"><path d="M2 4l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
        `;

        const dropdown = document.createElement('div');
        dropdown.className = 'palette-switcher-dropdown hidden';
        dropdown.id = 'palette-switcher-dropdown';
        dropdown.setAttribute('role', 'listbox');
        dropdown.innerHTML = allowedPalettes.map(p => `
            <button class="palette-switcher-item${p.key === currentNamedPalette ? ' active' : ''}"
                    data-palette-key="${p.key}"
                    role="option"
                    aria-selected="${p.key === currentNamedPalette}">
                <span class="palette-switcher-item-swatch">
                    <span class="palette-switcher-item-dot" style="background:${p.bg}"></span>
                    <span class="palette-switcher-item-dot" style="background:${p.accent}"></span>
                </span>
                <span class="palette-switcher-item-name">${esc(p.name)}</span>
            </button>
        `).join('');

        slot.appendChild(btn);
        slot.appendChild(dropdown);

        let open = false;

        btn.addEventListener('click', (e) => {
            e.stopPropagation();
            open = !open;
            dropdown.classList.toggle('hidden', !open);
        });

        dropdown.querySelectorAll('.palette-switcher-item').forEach(item => {
            item.addEventListener('click', () => {
                const key = item.dataset.paletteKey;
                open = false;
                dropdown.classList.add('hidden');
                applyNamedPalette(key);
            });
        });

        document.addEventListener('click', () => {
            if (open) {
                open = false;
                dropdown.classList.add('hidden');
            }
        });

        dropdown.addEventListener('click', (e) => e.stopPropagation());
    }

    // --- Init ---
    async function init() {
        // Fetch variant assignment
        try {
            const res = await fetch('/api/experiment/variant');
            const data = await res.json();
            currentVariant = data.variant;
        } catch (_) {
            const match = document.cookie.match(/co_variant=([a-h])/);
            if (match) currentVariant = match[1];
        }

        // Fetch theme tier — determines which palettes, variants, and editors are shown
        try {
            const res = await fetch('/api/v1/themes/available');
            if (res.ok) {
                availableThemes = await res.json();
            }
        } catch (_) { /* keep free-tier defaults */ }

        loadNamedPalette();
        loadPalette();
        renderWidget();
        renderHeaderSwitcher();
    }

    // --- Render ---
    function renderWidget() {
        const hasVariants = availableThemes.variants.length > 0;
        const hasCustomPalette = !!availableThemes.custom;

        // Pill — hide variant switch and palette editor for anonymous users
        const pill = document.createElement('div');
        pill.className = 'experiment-pill';
        pill.innerHTML = `
            <span class="experiment-pill-variant">Variant ${currentVariant.toUpperCase()}</span>
            ${hasVariants ? `
            <span class="experiment-pill-sep">|</span>
            <button class="experiment-pill-btn" id="exp-switch">Switch</button>
            ` : ''}
            ${hasCustomPalette ? `
            <span class="experiment-pill-sep">|</span>
            <button class="experiment-pill-btn" id="exp-palette">Palette</button>
            ` : ''}
            <span class="experiment-pill-sep">|</span>
            <button class="experiment-pill-btn" id="exp-feedback">Feedback</button>
        `;
        document.body.appendChild(pill);

        // Variant dropdown — only rendered for logged-in users
        if (hasVariants) {
            const dropdown = document.createElement('div');
            dropdown.className = 'experiment-dropdown hidden';
            dropdown.id = 'exp-dropdown';
            dropdown.innerHTML = VARIANTS.map(v => `
                <button class="experiment-dropdown-item${v.key === currentVariant ? ' active' : ''}" data-variant="${v.key}">
                    <span class="experiment-dropdown-item-letter">${v.key.toUpperCase()}</span>
                    ${esc(v.name)}
                </button>
            `).join('');
            document.body.appendChild(dropdown);
        }

        // Custom palette panel — only rendered for logged-in users
        if (hasCustomPalette) {
            const palettePanel = document.createElement('div');
            palettePanel.className = 'experiment-palette hidden';
            palettePanel.id = 'exp-palette-panel';
            palettePanel.innerHTML = `
                <div class="experiment-palette-header">
                    <span class="experiment-palette-title">Customize Palette</span>
                    <button class="experiment-palette-reset" id="exp-palette-reset">Reset</button>
                </div>
                <div class="experiment-palette-colors">
                    ${PALETTE_KEYS.map(p => `
                        <div class="experiment-palette-row">
                            <label>${esc(p.label)}</label>
                            <input type="color" data-var="${p.key}" class="experiment-palette-input">
                        </div>
                    `).join('')}
                </div>
            `;
            document.body.appendChild(palettePanel);
        }

        // Feedback overlay
        const overlay = document.createElement('div');
        overlay.className = 'experiment-feedback-overlay hidden';
        overlay.id = 'exp-feedback-overlay';
        overlay.innerHTML = `
            <div class="experiment-feedback">
                <div class="experiment-feedback-header">
                    <h3>Experiment Feedback</h3>
                    <button class="experiment-feedback-close" id="exp-feedback-close">&times;</button>
                </div>
                <form class="experiment-feedback-form" id="exp-feedback-form">
                    <div class="experiment-feedback-group">
                        <label>Rating (1-5)</label>
                        <div class="experiment-rating" id="exp-rating">
                            ${[1,2,3,4,5].map(n => `<button type="button" class="experiment-rating-btn" data-rating="${n}">${n}</button>`).join('')}
                        </div>
                    </div>
                    <div class="experiment-feedback-group">
                        <label>Preferred variant</label>
                        <div class="experiment-variant-radio">
                            ${VARIANTS.map(v => `
                                <div class="experiment-variant-option">
                                    <input type="radio" name="preferred" value="${v.key}" id="pref-${v.key}"${v.key === currentVariant ? ' checked' : ''}>
                                    <label for="pref-${v.key}">${v.key.toUpperCase()}</label>
                                </div>
                            `).join('')}
                        </div>
                    </div>
                    <div class="experiment-feedback-group">
                        <label>
                            <input type="checkbox" id="exp-include-palette"> Include my custom palette
                        </label>
                    </div>
                    <div class="experiment-feedback-group">
                        <label for="exp-comments">Comments</label>
                        <textarea id="exp-comments" placeholder="What did you think of this variant?"></textarea>
                    </div>
                    <div class="experiment-feedback-actions">
                        <button type="button" class="experiment-btn experiment-btn-secondary" id="exp-feedback-cancel">Cancel</button>
                        <button type="submit" class="experiment-btn experiment-btn-primary">Submit</button>
                    </div>
                </form>
            </div>
        `;
        document.body.appendChild(overlay);

        // Toast
        const toast = document.createElement('div');
        toast.className = 'experiment-toast hidden';
        toast.id = 'exp-toast';
        document.body.appendChild(toast);

        setupEvents();
        syncPaletteInputs();
    }

    // --- Sync palette color pickers with current values ---
    function syncPaletteInputs() {
        document.querySelectorAll('.experiment-palette-input').forEach(input => {
            const varName = input.dataset.var;
            let val = getComputedPaletteValue(varName);
            // color inputs need hex format
            if (val && !val.startsWith('#')) {
                // try to convert
                const temp = document.createElement('div');
                temp.style.color = val;
                document.body.appendChild(temp);
                const computed = getComputedStyle(temp).color;
                document.body.removeChild(temp);
                const match = computed.match(/(\d+),\s*(\d+),\s*(\d+)/);
                if (match) {
                    val = '#' + [match[1], match[2], match[3]].map(n => parseInt(n).toString(16).padStart(2, '0')).join('');
                }
            }
            if (val) input.value = val;
        });
    }

    // --- Events ---
    function setupEvents() {
        const hasVariants = availableThemes.variants.length > 0;
        const hasCustomPalette = !!availableThemes.custom;

        let dropdownOpen = false;
        let paletteOpen = false;

        if (hasVariants) {
            document.getElementById('exp-switch').addEventListener('click', (e) => {
                e.stopPropagation();
                dropdownOpen = !dropdownOpen;
                paletteOpen = false;
                document.getElementById('exp-dropdown').classList.toggle('hidden', !dropdownOpen);
                if (hasCustomPalette) {
                    document.getElementById('exp-palette-panel').classList.add('hidden');
                }
            });

            document.getElementById('exp-dropdown').querySelectorAll('.experiment-dropdown-item').forEach(btn => {
                btn.addEventListener('click', async () => {
                    const variant = btn.dataset.variant;
                    dropdownOpen = false;
                    document.getElementById('exp-dropdown').classList.add('hidden');
                    if (variant !== currentVariant) {
                        await applyVariantSwitch(variant);
                    }
                });
            });
        }

        if (hasCustomPalette) {
            document.getElementById('exp-palette').addEventListener('click', (e) => {
                e.stopPropagation();
                paletteOpen = !paletteOpen;
                dropdownOpen = false;
                document.getElementById('exp-palette-panel').classList.toggle('hidden', !paletteOpen);
                if (hasVariants) {
                    document.getElementById('exp-dropdown').classList.add('hidden');
                }
                if (paletteOpen) syncPaletteInputs();
            });

            // Palette color inputs
            document.getElementById('exp-palette-panel').addEventListener('click', e => e.stopPropagation());
            document.querySelectorAll('.experiment-palette-input').forEach(input => {
                input.addEventListener('input', () => {
                    const varName = input.dataset.var;
                    document.documentElement.style.setProperty(varName, input.value);
                    const palette = getCurrentPalette();
                    palette[varName] = input.value;
                    savePalette(palette);
                });
            });

            document.getElementById('exp-palette-reset').addEventListener('click', () => {
                resetPalette();
                window.location.reload();
            });
        }

        document.addEventListener('click', () => {
            if (dropdownOpen) {
                dropdownOpen = false;
                if (hasVariants) document.getElementById('exp-dropdown').classList.add('hidden');
            }
            if (paletteOpen) {
                paletteOpen = false;
                if (hasCustomPalette) document.getElementById('exp-palette-panel').classList.add('hidden');
            }
        });

        document.getElementById('exp-feedback').addEventListener('click', (e) => {
            e.stopPropagation();
            document.getElementById('exp-feedback-overlay').classList.remove('hidden');
        });

        const closeOverlay = () => {
            document.getElementById('exp-feedback-overlay').classList.add('hidden');
        };

        document.getElementById('exp-feedback-close').addEventListener('click', closeOverlay);
        document.getElementById('exp-feedback-cancel').addEventListener('click', closeOverlay);
        document.getElementById('exp-feedback-overlay').addEventListener('click', (e) => {
            if (e.target === e.currentTarget) closeOverlay();
        });

        // Rating buttons
        let selectedRating = 0;
        document.getElementById('exp-rating').querySelectorAll('.experiment-rating-btn').forEach(btn => {
            btn.addEventListener('click', () => {
                selectedRating = parseInt(btn.dataset.rating);
                document.getElementById('exp-rating').querySelectorAll('.experiment-rating-btn').forEach(b => {
                    b.classList.toggle('active', parseInt(b.dataset.rating) <= selectedRating);
                });
            });
        });

        // Form submit
        document.getElementById('exp-feedback-form').addEventListener('submit', async (e) => {
            e.preventDefault();
            if (selectedRating === 0) {
                showToast('Please select a rating');
                return;
            }

            const preferred = document.querySelector('input[name="preferred"]:checked');
            const comments = document.getElementById('exp-comments').value.trim();
            const includePalette = document.getElementById('exp-include-palette').checked;

            const body = {
                rating: selectedRating,
                preferred_variant: preferred ? preferred.value : currentVariant,
                comments,
            };

            if (includePalette) {
                const palette = getCurrentPalette();
                if (Object.keys(palette).length > 0) {
                    body.custom_palette = {
                        bg: palette['--bg'] || null,
                        sidebar_bg: palette['--sidebar-bg'] || null,
                        accent: palette['--accent'] || null,
                        text_primary: palette['--text-primary'] || null,
                        card_bg: palette['--card-bg'] || null,
                    };
                }
            }

            try {
                await fetch('/api/experiment/feedback', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify(body),
                });
                closeOverlay();
                showToast('Feedback submitted! Thank you.');
                selectedRating = 0;
                document.getElementById('exp-rating').querySelectorAll('.experiment-rating-btn').forEach(b => b.classList.remove('active'));
                document.getElementById('exp-comments').value = '';
            } catch (err) {
                showToast('Failed to submit feedback');
            }
        });
    }

    // --- Helpers ---
    function esc(s) {
        const d = document.createElement('div');
        d.textContent = s;
        return d.innerHTML;
    }

    function showToast(msg) {
        const toast = document.getElementById('exp-toast');
        toast.textContent = msg;
        toast.classList.remove('hidden');
        setTimeout(() => toast.classList.add('hidden'), 3000);
    }

    init();
})();
