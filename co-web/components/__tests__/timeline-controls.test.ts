import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

const MIN_ZOOM = 1
const MAX_ZOOM = 4

interface TimelineState {
  zoom: number
  periodIndex: number
}

const PERIODS = ['Jan 2026', 'Feb 2026', 'Mar 2026', 'Apr 2026', 'May 2026', 'Jun 2026']

function buildDOM() {
  document.body.innerHTML = `
    <div id="timeline">
      <button id="btn-zoom-in">+</button>
      <button id="btn-zoom-out">-</button>
      <button id="btn-prev">&lt;</button>
      <button id="btn-next">&gt;</button>
      <button id="btn-today">Today</button>
      <span class="timeline-period-label">${PERIODS[2]}</span>
    </div>
  `
}

function makeState(): TimelineState {
  return { zoom: 2, periodIndex: 2 }
}

function applyState(state: TimelineState) {
  const label = document.querySelector('.timeline-period-label') as HTMLElement
  label.textContent = PERIODS[state.periodIndex] || PERIODS[2]
  const timeline = document.getElementById('timeline') as HTMLElement
  timeline.dataset.zoom = String(state.zoom)
}

function attachListeners(state: TimelineState) {
  document.getElementById('btn-zoom-in')!.addEventListener('click', () => {
    state.zoom = Math.min(MAX_ZOOM, state.zoom + 1)
    const timeline = document.getElementById('timeline') as HTMLElement
    timeline.classList.add('zoom-in')
    applyState(state)
  })
  document.getElementById('btn-zoom-out')!.addEventListener('click', () => {
    state.zoom = Math.max(MIN_ZOOM, state.zoom - 1)
    const timeline = document.getElementById('timeline') as HTMLElement
    timeline.classList.remove('zoom-in')
    applyState(state)
  })
  document.getElementById('btn-prev')!.addEventListener('click', () => {
    state.periodIndex = Math.max(0, state.periodIndex - 1)
    applyState(state)
  })
  document.getElementById('btn-next')!.addEventListener('click', () => {
    state.periodIndex = Math.min(PERIODS.length - 1, state.periodIndex + 1)
    applyState(state)
  })
  document.getElementById('btn-today')!.addEventListener('click', () => {
    state.periodIndex = 2
    applyState(state)
  })
}

describe('timeline-controls', () => {
  let state: TimelineState

  beforeEach(() => {
    buildDOM()
    state = makeState()
    attachListeners(state)
    applyState(state)
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('initial period label is set', () => {
    const label = document.querySelector('.timeline-period-label') as HTMLElement
    expect(label.textContent).toBe(PERIODS[2])
  })

  it('clicking zoom-in adds zoom-in class to timeline', () => {
    ;(document.getElementById('btn-zoom-in') as HTMLButtonElement).click()
    expect(document.getElementById('timeline')!.classList.contains('zoom-in')).toBe(true)
  })

  it('clicking zoom-in increases zoom level', () => {
    ;(document.getElementById('btn-zoom-in') as HTMLButtonElement).click()
    expect(state.zoom).toBe(3)
  })

  it('clicking zoom-out decreases zoom level', () => {
    ;(document.getElementById('btn-zoom-out') as HTMLButtonElement).click()
    expect(state.zoom).toBe(1)
  })

  it('zoom clamps at max', () => {
    for (let i = 0; i < 10; i++) {
      ;(document.getElementById('btn-zoom-in') as HTMLButtonElement).click()
    }
    expect(state.zoom).toBe(MAX_ZOOM)
  })

  it('zoom clamps at min', () => {
    for (let i = 0; i < 10; i++) {
      ;(document.getElementById('btn-zoom-out') as HTMLButtonElement).click()
    }
    expect(state.zoom).toBe(MIN_ZOOM)
  })

  it('clicking next changes the period label forward', () => {
    ;(document.getElementById('btn-next') as HTMLButtonElement).click()
    const label = document.querySelector('.timeline-period-label') as HTMLElement
    expect(label.textContent).toBe(PERIODS[3])
  })

  it('clicking prev changes the period label backward', () => {
    ;(document.getElementById('btn-prev') as HTMLButtonElement).click()
    const label = document.querySelector('.timeline-period-label') as HTMLElement
    expect(label.textContent).toBe(PERIODS[1])
  })

  it('clicking today resets to current period', () => {
    ;(document.getElementById('btn-next') as HTMLButtonElement).click()
    ;(document.getElementById('btn-next') as HTMLButtonElement).click()
    ;(document.getElementById('btn-today') as HTMLButtonElement).click()
    const label = document.querySelector('.timeline-period-label') as HTMLElement
    expect(label.textContent).toBe(PERIODS[2])
  })

  it('period does not go before first', () => {
    for (let i = 0; i < 10; i++) {
      ;(document.getElementById('btn-prev') as HTMLButtonElement).click()
    }
    expect(state.periodIndex).toBe(0)
  })
})
