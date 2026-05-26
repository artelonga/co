import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

function buildDOM() {
  document.body.innerHTML = `
    <div class="task-card" data-id="p1">
      <button class="subtask-toggle" data-parent="p1">
        <span class="subtask-count">3</span>
      </button>
      <div class="subtask-row" data-parent="p1" style="display:none;padding-left:16px;">Child 1</div>
      <div class="subtask-row" data-parent="p1" style="display:none;padding-left:16px;">Child 2</div>
      <div class="subtask-row" data-parent="p1" style="display:none;padding-left:16px;">Child 3</div>
    </div>
    <div class="task-card" data-id="p2">
      <button class="subtask-toggle" data-parent="p2">
        <span class="subtask-count">2</span>
      </button>
      <div class="subtask-row" data-parent="p2" style="display:none;padding-left:16px;">Child A</div>
      <div class="subtask-row" data-parent="p2" style="display:none;padding-left:16px;">Child B</div>
    </div>
  `
}

const expandedState: Record<string, boolean> = {}

function toggleSubtasks(parentId: string) {
  const rows = document.querySelectorAll(`.subtask-row[data-parent="${parentId}"]`) as NodeListOf<HTMLElement>
  expandedState[parentId] = !expandedState[parentId]
  rows.forEach(row => {
    row.style.display = expandedState[parentId] ? '' : 'none'
  })
  try {
    localStorage.setItem(`subtask-expanded-${parentId}`, String(expandedState[parentId]))
  } catch { /* ignore */ }
}

function attachListeners() {
  document.querySelectorAll('.subtask-toggle').forEach(btn => {
    btn.addEventListener('click', () => {
      const parentId = (btn as HTMLElement).dataset.parent!
      toggleSubtasks(parentId)
    })
  })
}

function visibleRows(parentId: string): HTMLElement[] {
  return Array.from(document.querySelectorAll(`.subtask-row[data-parent="${parentId}"]`)).filter(
    r => (r as HTMLElement).style.display !== 'none'
  ) as HTMLElement[]
}

describe('subtask-tree', () => {
  beforeEach(() => {
    buildDOM()
    delete expandedState['p1']
    delete expandedState['p2']
    attachListeners()
    localStorage.clear()
  })

  afterEach(() => {
    document.body.innerHTML = ''
    localStorage.clear()
  })

  it('subtask rows are hidden initially', () => {
    expect(visibleRows('p1').length).toBe(0)
  })

  it('clicking toggle expands subtasks for p1', () => {
    ;(document.querySelector('.subtask-toggle[data-parent="p1"]') as HTMLButtonElement).click()
    expect(visibleRows('p1').length).toBe(3)
  })

  it('clicking toggle twice collapses subtasks', () => {
    const btn = document.querySelector('.subtask-toggle[data-parent="p1"]') as HTMLButtonElement
    btn.click()
    btn.click()
    expect(visibleRows('p1').length).toBe(0)
  })

  it('toggle shows correct count badge for p1', () => {
    const badge = document.querySelector('.subtask-toggle[data-parent="p1"] .subtask-count') as HTMLElement
    expect(badge.textContent).toBe('3')
  })

  it('toggle shows correct count badge for p2', () => {
    const badge = document.querySelector('.subtask-toggle[data-parent="p2"] .subtask-count') as HTMLElement
    expect(badge.textContent).toBe('2')
  })

  it('nested rows have indentation style', () => {
    ;(document.querySelector('.subtask-toggle[data-parent="p1"]') as HTMLButtonElement).click()
    const rows = document.querySelectorAll('.subtask-row[data-parent="p1"]') as NodeListOf<HTMLElement>
    rows.forEach(row => {
      expect(row.style.paddingLeft).toBeTruthy()
    })
  })

  it('expanding p2 does not affect p1 rows', () => {
    ;(document.querySelector('.subtask-toggle[data-parent="p2"]') as HTMLButtonElement).click()
    expect(visibleRows('p1').length).toBe(0)
  })

  it('expanding p1 does not affect p2 rows', () => {
    ;(document.querySelector('.subtask-toggle[data-parent="p1"]') as HTMLButtonElement).click()
    expect(visibleRows('p2').length).toBe(0)
  })

  it('multiple parents can be expanded independently', () => {
    ;(document.querySelector('.subtask-toggle[data-parent="p1"]') as HTMLButtonElement).click()
    ;(document.querySelector('.subtask-toggle[data-parent="p2"]') as HTMLButtonElement).click()
    expect(visibleRows('p1').length).toBe(3)
    expect(visibleRows('p2').length).toBe(2)
  })

  it('toggle state is stored in localStorage', () => {
    ;(document.querySelector('.subtask-toggle[data-parent="p1"]') as HTMLButtonElement).click()
    expect(localStorage.getItem('subtask-expanded-p1')).toBe('true')
  })

  it('collapsing stores false in localStorage', () => {
    const btn = document.querySelector('.subtask-toggle[data-parent="p1"]') as HTMLButtonElement
    btn.click()
    btn.click()
    expect(localStorage.getItem('subtask-expanded-p1')).toBe('false')
  })

  it('p2 has two subtask rows total', () => {
    expect(document.querySelectorAll('.subtask-row[data-parent="p2"]').length).toBe(2)
  })
})
