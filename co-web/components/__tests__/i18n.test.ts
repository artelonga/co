import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

const translations: Record<string, Record<string, string>> = {
  pt: {
    'nav.projects': 'Projetos',
    'board.todo': 'A fazer',
    'board.done': 'Concluído',
    'action.new': 'Nova Tarefa',
  },
  en: {
    'nav.projects': 'Projects',
    'board.todo': 'To do',
    'board.done': 'Done',
    'action.new': 'New Task',
  },
}

function buildDOM() {
  document.body.innerHTML = `
    <button id="lang-toggle" data-lang="pt">EN</button>
    <span data-i18n="nav.projects">Projetos</span>
    <span data-i18n="board.todo">A fazer</span>
    <span data-i18n="board.done">Concluído</span>
    <button data-i18n="action.new">Nova Tarefa</button>
  `
}

function currentLang(): string {
  return (document.getElementById('lang-toggle') as HTMLButtonElement).dataset.lang || 'pt'
}

function applyLang(lang: string) {
  const toggle = document.getElementById('lang-toggle') as HTMLButtonElement
  toggle.dataset.lang = lang
  toggle.textContent = lang === 'pt' ? 'EN' : 'PT'
  document.querySelectorAll('[data-i18n]').forEach(el => {
    const key = (el as HTMLElement).dataset.i18n!
    const text = translations[lang]?.[key]
    if (text) el.textContent = text
  })
  try { localStorage.setItem('lang', lang) } catch { /* ignore */ }
}

function attachListeners() {
  document.getElementById('lang-toggle')!.addEventListener('click', () => {
    const next = currentLang() === 'pt' ? 'en' : 'pt'
    applyLang(next)
  })
}

describe('i18n', () => {
  beforeEach(() => {
    localStorage.clear()
    buildDOM()
    attachListeners()
  })

  afterEach(() => {
    document.body.innerHTML = ''
    localStorage.clear()
  })

  it('initial language is pt', () => {
    expect(currentLang()).toBe('pt')
  })

  it('initial labels are in Portuguese', () => {
    const span = document.querySelector('[data-i18n="nav.projects"]') as HTMLElement
    expect(span.textContent).toBe('Projetos')
  })

  it('clicking toggle switches language to en', () => {
    ;(document.getElementById('lang-toggle') as HTMLButtonElement).click()
    expect(currentLang()).toBe('en')
  })

  it('data-i18n elements update to English after toggle', () => {
    ;(document.getElementById('lang-toggle') as HTMLButtonElement).click()
    const span = document.querySelector('[data-i18n="nav.projects"]') as HTMLElement
    expect(span.textContent).toBe('Projects')
  })

  it('board.todo updates to English', () => {
    ;(document.getElementById('lang-toggle') as HTMLButtonElement).click()
    const span = document.querySelector('[data-i18n="board.todo"]') as HTMLElement
    expect(span.textContent).toBe('To do')
  })

  it('language is stored in localStorage after toggle', () => {
    ;(document.getElementById('lang-toggle') as HTMLButtonElement).click()
    expect(localStorage.getItem('lang')).toBe('en')
  })

  it('switching back to pt restores Portuguese labels', () => {
    const btn = document.getElementById('lang-toggle') as HTMLButtonElement
    btn.click()
    btn.click()
    const span = document.querySelector('[data-i18n="board.done"]') as HTMLElement
    expect(span.textContent).toBe('Concluído')
  })

  it('toggle button label shows EN when language is pt', () => {
    const btn = document.getElementById('lang-toggle') as HTMLButtonElement
    expect(btn.textContent).toBe('EN')
  })
})

// ── CO-556: real i18n.js — pt/en parity + toggle content switch ──────────────
// Loads the actual shipped dictionary (not the stub above) and asserts the
// invariants the SPA chrome relies on.
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'

function loadI18n(pathname: string) {
  const here = dirname(fileURLToPath(import.meta.url))
  const src = readFileSync(resolve(here, '../../static/shared/i18n.js'), 'utf8')
  let navigated: string | null = null
  const win: any = {
    location: { pathname, assign: (u: string) => { navigated = u } },
  }
  const doc: any = {
    cookie: '',
    documentElement: {},
    querySelectorAll: () => [],
    dispatchEvent: () => {},
    addEventListener: () => {},
  }
  // eslint-disable-next-line no-new-func
  new Function('window', 'document', src)(win, doc)
  return { win, getNav: () => navigated }
}

describe('CO-556: shipped i18n.js', () => {
  it('pt and en maps have identical key sets (no missing/leaking keys)', () => {
    const { win } = loadI18n('/')
    const pt = Object.keys(win.I18N.pt)
    const en = Object.keys(win.I18N.en)
    const ptSet = new Set(pt)
    const enSet = new Set(en)
    expect(pt.filter(k => !enSet.has(k))).toEqual([])
    expect(en.filter(k => !ptSet.has(k))).toEqual([])
  })

  it('no key resolves to a blank value in either language', () => {
    const { win } = loadI18n('/')
    for (const lang of ['pt', 'en']) {
      const blank = Object.keys(win.I18N[lang]).filter(k => !win.I18N[lang][k])
      expect(blank).toEqual([])
    }
  })

  it('toggling on a content page navigates to the language counterpart', () => {
    const { win, getNav } = loadI18n('/sobre')
    expect(win.currentLang).toBe('pt')
    win.setLang('en')
    expect(getNav()).toBe('/en/sobre')
  })

  it('an /en/ content URL is authoritative for the chrome language', () => {
    const { win, getNav } = loadI18n('/en/sobre')
    expect(win.currentLang).toBe('en')
    // re-setting the same language must not loop/navigate
    win.setLang('en')
    expect(getNav()).toBeNull()
  })

  it('toggling on the board SPA does not redirect', () => {
    const { win, getNav } = loadI18n('/')
    win.setLang('en')
    expect(getNav()).toBeNull()
  })
})
