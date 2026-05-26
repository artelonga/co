import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

const BREAKPOINT = 640

function buildDOM() {
  document.body.innerHTML = `
    <div id="app-layout">
      <aside id="sidebar" class="sidebar"></aside>
      <button id="hamburger-btn" class="hamburger" style="display:none;"></button>
      <main id="main-content"></main>
    </div>
  `
}

function applyLayout(width: number) {
  const sidebar = document.getElementById('sidebar') as HTMLElement
  const hamburger = document.getElementById('hamburger-btn') as HTMLElement
  if (width < BREAKPOINT) {
    sidebar.classList.add('sidebar-mobile')
    sidebar.classList.remove('sidebar-desktop')
    hamburger.style.display = 'block'
    if (!sidebar.classList.contains('open')) {
      sidebar.style.display = 'none'
    }
  } else {
    sidebar.classList.remove('sidebar-mobile')
    sidebar.classList.add('sidebar-desktop')
    hamburger.style.display = 'none'
    sidebar.style.display = ''
  }
}

function toggleSidebar() {
  const sidebar = document.getElementById('sidebar') as HTMLElement
  if (sidebar.classList.contains('open')) {
    sidebar.classList.remove('open')
    sidebar.style.display = 'none'
  } else {
    sidebar.classList.add('open')
    sidebar.style.display = 'block'
  }
}

function attachListeners() {
  document.getElementById('hamburger-btn')!.addEventListener('click', toggleSidebar)
}

describe('responsive-layout', () => {
  beforeEach(() => {
    buildDOM()
    attachListeners()
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('narrow viewport hides sidebar', () => {
    applyLayout(375)
    const sidebar = document.getElementById('sidebar') as HTMLElement
    expect(sidebar.style.display).toBe('none')
  })

  it('narrow viewport shows hamburger button', () => {
    applyLayout(375)
    const hamburger = document.getElementById('hamburger-btn') as HTMLElement
    expect(hamburger.style.display).toBe('block')
  })

  it('wide viewport shows sidebar', () => {
    applyLayout(1024)
    const sidebar = document.getElementById('sidebar') as HTMLElement
    expect(sidebar.style.display).toBe('')
  })

  it('wide viewport hides hamburger button', () => {
    applyLayout(1024)
    const hamburger = document.getElementById('hamburger-btn') as HTMLElement
    expect(hamburger.style.display).toBe('none')
  })

  it('narrow viewport adds sidebar-mobile class', () => {
    applyLayout(320)
    expect(document.getElementById('sidebar')!.classList.contains('sidebar-mobile')).toBe(true)
  })

  it('wide viewport adds sidebar-desktop class', () => {
    applyLayout(1280)
    expect(document.getElementById('sidebar')!.classList.contains('sidebar-desktop')).toBe(true)
  })

  it('hamburger click on narrow viewport opens sidebar', () => {
    applyLayout(375)
    ;(document.getElementById('hamburger-btn') as HTMLButtonElement).click()
    const sidebar = document.getElementById('sidebar') as HTMLElement
    expect(sidebar.classList.contains('open')).toBe(true)
  })

  it('hamburger click on narrow viewport makes sidebar visible', () => {
    applyLayout(375)
    ;(document.getElementById('hamburger-btn') as HTMLButtonElement).click()
    const sidebar = document.getElementById('sidebar') as HTMLElement
    expect(sidebar.style.display).toBe('block')
  })
})
