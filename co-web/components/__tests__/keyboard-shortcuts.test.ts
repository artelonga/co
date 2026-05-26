import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

type View = 'kanban' | 'table' | 'timeline' | 'calendar' | 'dashboard' | 'content'
const VIEW_KEYS: Record<string, View> = {
  '1': 'kanban',
  '2': 'table',
  '3': 'timeline',
  '4': 'calendar',
  '5': 'dashboard',
  '6': 'content',
}

function buildDOM() {
  document.body.innerHTML = `
    <button id="tab-kanban" class="tab active" data-view="kanban"></button>
    <button id="tab-table" class="tab" data-view="table"></button>
    <button id="tab-timeline" class="tab" data-view="timeline"></button>
    <button id="tab-calendar" class="tab" data-view="calendar"></button>
    <button id="tab-dashboard" class="tab" data-view="dashboard"></button>
    <button id="tab-content" class="tab" data-view="content"></button>
    <div id="new-task-modal" style="display:none;"></div>
    <input id="search-input" type="text" />
    <input id="other-input" type="text" />
    <textarea id="some-textarea"></textarea>
  `
  document.body.setAttribute('data-view', 'kanban')
}

function handleKey(e: KeyboardEvent) {
  const target = e.target as HTMLElement
  if (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA') return
  if (VIEW_KEYS[e.key]) {
    const view = VIEW_KEYS[e.key]
    document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'))
    const btn = document.getElementById(`tab-${view}`) as HTMLElement
    btn.classList.add('active')
    document.body.setAttribute('data-view', view)
  }
  if (e.key === 'n') {
    const modal = document.getElementById('new-task-modal')
    if (modal) modal.style.display = 'block'
  }
  if (e.key === '/') {
    e.preventDefault()
    const search = document.getElementById('search-input') as HTMLInputElement
    if (search) search.focus()
  }
  if (e.key === 'Escape') {
    const modal = document.getElementById('new-task-modal')
    if (modal) modal.style.display = 'none'
  }
}

describe('keyboard-shortcuts', () => {
  beforeEach(() => {
    buildDOM()
    document.addEventListener('keydown', handleKey)
  })

  afterEach(() => {
    document.removeEventListener('keydown', handleKey)
    document.body.innerHTML = ''
    document.body.removeAttribute('data-view')
  })

  it('pressing 1 switches to kanban view', () => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '1', bubbles: true }))
    expect(document.body.getAttribute('data-view')).toBe('kanban')
  })

  it('pressing 2 switches to table view', () => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '2', bubbles: true }))
    expect(document.body.getAttribute('data-view')).toBe('table')
  })

  it('pressing 3 switches to timeline view', () => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '3', bubbles: true }))
    expect(document.body.getAttribute('data-view')).toBe('timeline')
  })

  it('pressing 4 switches to calendar view', () => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '4', bubbles: true }))
    expect(document.body.getAttribute('data-view')).toBe('calendar')
  })

  it('pressing 5 switches to dashboard view', () => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '5', bubbles: true }))
    expect(document.body.getAttribute('data-view')).toBe('dashboard')
  })

  it('pressing 6 switches to content view', () => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '6', bubbles: true }))
    expect(document.body.getAttribute('data-view')).toBe('content')
  })

  it('pressing n opens new-task-modal', () => {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'n', bubbles: true }))
    const modal = document.getElementById('new-task-modal') as HTMLElement
    expect(modal.style.display).toBe('block')
  })

  it('pressing Escape closes new-task-modal', () => {
    const modal = document.getElementById('new-task-modal') as HTMLElement
    modal.style.display = 'block'
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(modal.style.display).toBe('none')
  })

  it('pressing / focuses search input', () => {
    const search = document.getElementById('search-input') as HTMLInputElement
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true }))
    expect(document.activeElement).toBe(search)
  })

  it('keypresses inside an input element are ignored for view switching', () => {
    const input = document.getElementById('other-input') as HTMLInputElement
    input.focus()
    const event = new KeyboardEvent('keydown', { key: '2', bubbles: true })
    Object.defineProperty(event, 'target', { value: input })
    handleKey(event)
    expect(document.body.getAttribute('data-view')).toBe('kanban')
  })
})
