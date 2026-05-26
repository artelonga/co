import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

type View = 'kanban' | 'table' | 'timeline' | 'calendar' | 'dashboard' | 'content'

const VIEWS: View[] = ['kanban', 'table', 'timeline', 'calendar', 'dashboard', 'content']

function buildDOM() {
  document.body.innerHTML = `
    <div id="view-tabs">
      <button id="tab-kanban" data-view="kanban" aria-selected="true" class="tab active">Kanban</button>
      <button id="tab-table" data-view="table" aria-selected="false" class="tab">Table</button>
      <button id="tab-timeline" data-view="timeline" aria-selected="false" class="tab">Timeline</button>
      <button id="tab-calendar" data-view="calendar" aria-selected="false" class="tab">Calendar</button>
      <button id="tab-dashboard" data-view="dashboard" aria-selected="false" class="tab">Dashboard</button>
      <button id="tab-content" data-view="content" aria-selected="false" class="tab">Content</button>
    </div>
    <div id="view-area" data-view="kanban"></div>
  `
  document.body.setAttribute('data-view', 'kanban')
}

function switchTab(view: View) {
  const btn = document.getElementById(`tab-${view}`) as HTMLButtonElement
  document.querySelectorAll('.tab').forEach(t => {
    t.classList.remove('active')
    t.setAttribute('aria-selected', 'false')
  })
  btn.classList.add('active')
  btn.setAttribute('aria-selected', 'true')
  document.body.setAttribute('data-view', view)
  const area = document.getElementById('view-area') as HTMLElement
  area.setAttribute('data-view', view)
}

VIEWS.forEach(view => {
  document.getElementById(`tab-${view}`)?.addEventListener('click', () => switchTab(view))
})

describe('view-tabs', () => {
  beforeEach(() => {
    buildDOM()
    VIEWS.forEach(view => {
      const btn = document.getElementById(`tab-${view}`) as HTMLButtonElement
      btn.addEventListener('click', () => switchTab(view))
    })
  })

  afterEach(() => {
    document.body.innerHTML = ''
    document.body.removeAttribute('data-view')
  })

  it('initial state has kanban tab active', () => {
    const btn = document.getElementById('tab-kanban') as HTMLButtonElement
    expect(btn.classList.contains('active')).toBe(true)
  })

  it('initial body data-view is kanban', () => {
    expect(document.body.getAttribute('data-view')).toBe('kanban')
  })

  it('clicking table tab sets it active', () => {
    ;(document.getElementById('tab-table') as HTMLButtonElement).click()
    expect(document.getElementById('tab-table')!.classList.contains('active')).toBe(true)
  })

  it('clicking table tab removes active from kanban', () => {
    ;(document.getElementById('tab-table') as HTMLButtonElement).click()
    expect(document.getElementById('tab-kanban')!.classList.contains('active')).toBe(false)
  })

  it('clicking each view tab updates body data-view', () => {
    for (const view of VIEWS) {
      ;(document.getElementById(`tab-${view}`) as HTMLButtonElement).click()
      expect(document.body.getAttribute('data-view')).toBe(view)
    }
  })

  it('only one tab has active class at a time', () => {
    ;(document.getElementById('tab-timeline') as HTMLButtonElement).click()
    const active = document.querySelectorAll('.tab.active')
    expect(active.length).toBe(1)
  })

  it('clicking same tab twice is idempotent', () => {
    ;(document.getElementById('tab-calendar') as HTMLButtonElement).click()
    ;(document.getElementById('tab-calendar') as HTMLButtonElement).click()
    expect(document.getElementById('tab-calendar')!.classList.contains('active')).toBe(true)
    expect(document.body.getAttribute('data-view')).toBe('calendar')
  })

  it('aria-selected is true on active tab', () => {
    ;(document.getElementById('tab-dashboard') as HTMLButtonElement).click()
    expect(document.getElementById('tab-dashboard')!.getAttribute('aria-selected')).toBe('true')
  })

  it('aria-selected is false on inactive tabs', () => {
    ;(document.getElementById('tab-dashboard') as HTMLButtonElement).click()
    VIEWS.filter(v => v !== 'dashboard').forEach(v => {
      expect(document.getElementById(`tab-${v}`)!.getAttribute('aria-selected')).toBe('false')
    })
  })

  it('clicking content tab updates view-area data-view', () => {
    ;(document.getElementById('tab-content') as HTMLButtonElement).click()
    const area = document.getElementById('view-area') as HTMLElement
    expect(area.getAttribute('data-view')).toBe('content')
  })

  it('all 6 tab buttons are present', () => {
    VIEWS.forEach(view => {
      expect(document.getElementById(`tab-${view}`)).not.toBeNull()
    })
  })

  it('switching from one view to another clears previous aria-selected', () => {
    ;(document.getElementById('tab-table') as HTMLButtonElement).click()
    ;(document.getElementById('tab-timeline') as HTMLButtonElement).click()
    expect(document.getElementById('tab-table')!.getAttribute('aria-selected')).toBe('false')
  })
})
