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
