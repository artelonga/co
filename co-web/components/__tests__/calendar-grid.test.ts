import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

const DAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']
const MONTHS = [
  'January', 'February', 'March', 'April', 'May', 'June',
  'July', 'August', 'September', 'October', 'November', 'December',
]

interface CalState {
  year: number
  month: number // 0-indexed
  selectedDay: number | null
}

function daysInMonth(year: number, month: number): number {
  return new Date(year, month + 1, 0).getDate()
}

function buildDOM(state: CalState) {
  const count = daysInMonth(state.year, state.month)
  const todayDate = new Date()
  const isCurrentMonth = todayDate.getFullYear() === state.year && todayDate.getMonth() === state.month
  const todayDay = isCurrentMonth ? todayDate.getDate() : -1

  const headers = DAY_NAMES.map(d => `<div class="calendar-header-cell">${d}</div>`).join('')
  const days = Array.from({ length: count }, (_, i) => {
    const day = i + 1
    const isToday = day === todayDay
    const isSelected = day === state.selectedDay
    return `<div class="calendar-day${isToday ? ' today' : ''}${isSelected ? ' selected' : ''}" data-day="${day}">${day}</div>`
  }).join('')

  document.body.innerHTML = `
    <div id="calendar">
      <div id="calendar-nav">
        <button id="cal-prev">&lt;</button>
        <span id="calendar-month-label">${MONTHS[state.month]} ${state.year}</span>
        <button id="cal-next">&gt;</button>
      </div>
      <div id="calendar-grid">
        ${headers}
        ${days}
      </div>
    </div>
  `
}

function attachListeners(state: CalState) {
  document.getElementById('cal-prev')!.addEventListener('click', () => {
    state.month -= 1
    if (state.month < 0) { state.month = 11; state.year -= 1 }
    state.selectedDay = null
    buildDOM(state)
    attachListeners(state)
  })
  document.getElementById('cal-next')!.addEventListener('click', () => {
    state.month += 1
    if (state.month > 11) { state.month = 0; state.year += 1 }
    state.selectedDay = null
    buildDOM(state)
    attachListeners(state)
  })
  document.querySelectorAll('.calendar-day').forEach(cell => {
    cell.addEventListener('click', () => {
      state.selectedDay = parseInt((cell as HTMLElement).dataset.day!, 10)
      document.querySelectorAll('.calendar-day.selected').forEach(c => c.classList.remove('selected'))
      cell.classList.add('selected')
    })
  })
}

describe('calendar-grid', () => {
  let state: CalState

  beforeEach(() => {
    state = { year: 2026, month: 4, selectedDay: null } // May 2026
    buildDOM(state)
    attachListeners(state)
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('grid has 7 header columns', () => {
    expect(document.querySelectorAll('.calendar-header-cell').length).toBe(7)
  })

  it('header columns are the correct day names', () => {
    const headers = Array.from(document.querySelectorAll('.calendar-header-cell')).map(h => h.textContent)
    expect(headers).toEqual(DAY_NAMES)
  })

  it('May 2026 has 31 days', () => {
    expect(document.querySelectorAll('.calendar-day').length).toBe(31)
  })

  it('month label shows correct month', () => {
    expect(document.getElementById('calendar-month-label')!.textContent).toBe('May 2026')
  })

  it('clicking a day selects it', () => {
    ;(document.querySelector('.calendar-day[data-day="15"]') as HTMLElement).click()
    expect(document.querySelector('.calendar-day[data-day="15"]')!.classList.contains('selected')).toBe(true)
  })

  it('only one day is selected at a time', () => {
    ;(document.querySelector('.calendar-day[data-day="10"]') as HTMLElement).click()
    ;(document.querySelector('.calendar-day[data-day="20"]') as HTMLElement).click()
    expect(document.querySelectorAll('.calendar-day.selected').length).toBe(1)
  })

  it('clicking next month changes month label', () => {
    ;(document.getElementById('cal-next') as HTMLButtonElement).click()
    expect(document.getElementById('calendar-month-label')!.textContent).toBe('June 2026')
  })

  it('clicking prev month changes month label', () => {
    ;(document.getElementById('cal-prev') as HTMLButtonElement).click()
    expect(document.getElementById('calendar-month-label')!.textContent).toBe('April 2026')
  })

  it('February 2026 has 28 days', () => {
    state.month = 1
    buildDOM(state)
    attachListeners(state)
    expect(document.querySelectorAll('.calendar-day').length).toBe(28)
  })

  it('days in month range is 28-31', () => {
    const count = document.querySelectorAll('.calendar-day').length
    expect(count).toBeGreaterThanOrEqual(28)
    expect(count).toBeLessThanOrEqual(31)
  })
})
