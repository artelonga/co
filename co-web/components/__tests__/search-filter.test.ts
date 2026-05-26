import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

function buildDOM() {
  document.body.innerHTML = `
    <input id="search-input" type="text" />
    <div id="task-list">
      <div class="task-card" data-title="Fix authentication bug">Fix authentication bug</div>
      <div class="task-card" data-title="Add dark mode">Add dark mode</div>
      <div class="task-card" data-title="Write unit tests">Write unit tests</div>
      <div class="task-card" data-title="Deploy to production">Deploy to production</div>
      <div class="task-card" data-title="Update documentation">Update documentation</div>
    </div>
  `
}

function filterCards(query: string) {
  const cards = document.querySelectorAll('.task-card') as NodeListOf<HTMLElement>
  const q = query.trim().toLowerCase()
  cards.forEach(card => {
    const title = (card.dataset.title || '').toLowerCase()
    card.style.display = !q || title.includes(q) ? '' : 'none'
  })
}

function attachListeners() {
  const input = document.getElementById('search-input') as HTMLInputElement
  input.addEventListener('input', () => filterCards(input.value))
}

function visibleCards(): HTMLElement[] {
  return Array.from(document.querySelectorAll('.task-card')).filter(
    c => (c as HTMLElement).style.display !== 'none'
  ) as HTMLElement[]
}

describe('search-filter', () => {
  beforeEach(() => {
    buildDOM()
    attachListeners()
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('all cards are visible before any search', () => {
    expect(visibleCards().length).toBe(5)
  })

  it('typing a query hides non-matching cards', () => {
    const input = document.getElementById('search-input') as HTMLInputElement
    input.value = 'dark'
    input.dispatchEvent(new Event('input'))
    expect(visibleCards().length).toBe(1)
  })

  it('matching card remains visible', () => {
    const input = document.getElementById('search-input') as HTMLInputElement
    input.value = 'dark'
    input.dispatchEvent(new Event('input'))
    expect(visibleCards()[0].dataset.title).toBe('Add dark mode')
  })

  it('clearing input shows all cards', () => {
    const input = document.getElementById('search-input') as HTMLInputElement
    input.value = 'test'
    input.dispatchEvent(new Event('input'))
    input.value = ''
    input.dispatchEvent(new Event('input'))
    expect(visibleCards().length).toBe(5)
  })

  it('search is case-insensitive', () => {
    const input = document.getElementById('search-input') as HTMLInputElement
    input.value = 'FIX'
    input.dispatchEvent(new Event('input'))
    expect(visibleCards().length).toBe(1)
    expect(visibleCards()[0].dataset.title).toBe('Fix authentication bug')
  })

  it('partial match works', () => {
    const input = document.getElementById('search-input') as HTMLInputElement
    input.value = 'doc'
    input.dispatchEvent(new Event('input'))
    expect(visibleCards().length).toBe(1)
    expect(visibleCards()[0].dataset.title).toBe('Update documentation')
  })

  it('empty string shows all cards', () => {
    const input = document.getElementById('search-input') as HTMLInputElement
    input.value = ''
    input.dispatchEvent(new Event('input'))
    expect(visibleCards().length).toBe(5)
  })

  it('whitespace-only shows all cards', () => {
    const input = document.getElementById('search-input') as HTMLInputElement
    input.value = '   '
    input.dispatchEvent(new Event('input'))
    expect(visibleCards().length).toBe(5)
  })

  it('query with no matches hides all cards', () => {
    const input = document.getElementById('search-input') as HTMLInputElement
    input.value = 'zzznomatch'
    input.dispatchEvent(new Event('input'))
    expect(visibleCards().length).toBe(0)
  })

  it('multi-word partial match works', () => {
    const input = document.getElementById('search-input') as HTMLInputElement
    input.value = 'unit'
    input.dispatchEvent(new Event('input'))
    expect(visibleCards().length).toBe(1)
    expect(visibleCards()[0].dataset.title).toBe('Write unit tests')
  })
})
