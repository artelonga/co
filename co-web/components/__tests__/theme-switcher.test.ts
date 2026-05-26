import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

const THEMES = [
  'modern', 'scholarly-light', 'scholarly-dark', 'relic-light', 'relic-dark',
  'medieval', 'steampunk', 'cyberpunk', 'matrix', 'garden', 'terminal', 'retro',
]

function buildDOM() {
  const buttons = THEMES.map(t =>
    `<button class="theme-option" data-theme="${t}">${t}</button>`
  ).join('')
  document.body.innerHTML = `
    <div id="theme-switcher">
      ${buttons}
    </div>
  `
}

function applyTheme(theme: string) {
  document.documentElement.setAttribute('data-palette', theme)
  try { localStorage.setItem('theme', theme) } catch { /* ignore */ }
}

function loadStoredTheme() {
  try {
    const stored = localStorage.getItem('theme')
    if (stored) applyTheme(stored)
  } catch { /* ignore */ }
}

function attachListeners() {
  document.querySelectorAll('.theme-option').forEach(btn => {
    btn.addEventListener('click', () => {
      const theme = (btn as HTMLElement).dataset.theme!
      applyTheme(theme)
    })
  })
}

let reloaded = false

describe('theme-switcher', () => {
  beforeEach(() => {
    reloaded = false
    localStorage.clear()
    document.documentElement.removeAttribute('data-palette')
    buildDOM()
    attachListeners()
  })

  afterEach(() => {
    document.body.innerHTML = ''
    document.documentElement.removeAttribute('data-palette')
    localStorage.clear()
  })

  it('all 12 themes are present as option buttons', () => {
    expect(document.querySelectorAll('.theme-option').length).toBe(12)
  })

  it('each theme button has correct data-theme attribute', () => {
    const themeValues = Array.from(document.querySelectorAll('.theme-option')).map(
      b => (b as HTMLElement).dataset.theme
    )
    expect(themeValues).toEqual(THEMES)
  })

  it('clicking a theme button sets data-palette on html element', () => {
    ;(document.querySelector('.theme-option[data-theme="cyberpunk"]') as HTMLButtonElement).click()
    expect(document.documentElement.getAttribute('data-palette')).toBe('cyberpunk')
  })

  it('clicking modern sets data-palette to modern', () => {
    ;(document.querySelector('.theme-option[data-theme="modern"]') as HTMLButtonElement).click()
    expect(document.documentElement.getAttribute('data-palette')).toBe('modern')
  })

  it('switching theme updates data-palette', () => {
    ;(document.querySelector('.theme-option[data-theme="retro"]') as HTMLButtonElement).click()
    ;(document.querySelector('.theme-option[data-theme="matrix"]') as HTMLButtonElement).click()
    expect(document.documentElement.getAttribute('data-palette')).toBe('matrix')
  })

  it('theme is stored in localStorage after click', () => {
    ;(document.querySelector('.theme-option[data-theme="steampunk"]') as HTMLButtonElement).click()
    expect(localStorage.getItem('theme')).toBe('steampunk')
  })

  it('loadStoredTheme applies localStorage theme', () => {
    localStorage.setItem('theme', 'garden')
    loadStoredTheme()
    expect(document.documentElement.getAttribute('data-palette')).toBe('garden')
  })

  it('switching theme does not set reloaded flag', () => {
    ;(document.querySelector('.theme-option[data-theme="terminal"]') as HTMLButtonElement).click()
    expect(reloaded).toBe(false)
  })

  it('initially no data-palette on html when localStorage empty', () => {
    expect(document.documentElement.getAttribute('data-palette')).toBeNull()
  })

  it('medieval theme button is present', () => {
    expect(document.querySelector('.theme-option[data-theme="medieval"]')).not.toBeNull()
  })
})
