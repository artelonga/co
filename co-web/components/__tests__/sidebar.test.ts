import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

function buildDOM() {
  document.body.innerHTML = `
    <aside id="sidebar" class="sidebar">
      <ul id="project-list">
        <li class="project-item" data-key="alpha">Alpha</li>
        <li class="project-item" data-key="beta">Beta</li>
        <li class="project-item" data-key="gamma">Gamma</li>
      </ul>
    </aside>
    <button id="hamburger-btn" class="hamburger">&#9776;</button>
    <main id="main-content"></main>
  `
}

function selectProject(key: string) {
  document.querySelectorAll('.project-item').forEach(item => item.classList.remove('active'))
  const item = document.querySelector(`.project-item[data-key="${key}"]`)
  if (item) item.classList.add('active')
}

function toggleSidebar() {
  const sidebar = document.getElementById('sidebar') as HTMLElement
  sidebar.classList.toggle('open')
}

function attachListeners() {
  document.querySelectorAll('.project-item').forEach(item => {
    item.addEventListener('click', () => {
      selectProject((item as HTMLElement).dataset.key!)
    })
  })
  const hamburger = document.getElementById('hamburger-btn') as HTMLButtonElement
  hamburger.addEventListener('click', toggleSidebar)
}

describe('sidebar', () => {
  beforeEach(() => {
    buildDOM()
    attachListeners()
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('no project is active initially', () => {
    const active = document.querySelectorAll('.project-item.active')
    expect(active.length).toBe(0)
  })

  it('clicking a project sets .active class on it', () => {
    ;(document.querySelector('.project-item[data-key="alpha"]') as HTMLElement).click()
    expect(document.querySelector('.project-item[data-key="alpha"]')!.classList.contains('active')).toBe(true)
  })

  it('only one project is active at a time', () => {
    ;(document.querySelector('.project-item[data-key="alpha"]') as HTMLElement).click()
    ;(document.querySelector('.project-item[data-key="beta"]') as HTMLElement).click()
    expect(document.querySelectorAll('.project-item.active').length).toBe(1)
  })

  it('clicking beta removes active from alpha', () => {
    ;(document.querySelector('.project-item[data-key="alpha"]') as HTMLElement).click()
    ;(document.querySelector('.project-item[data-key="beta"]') as HTMLElement).click()
    expect(document.querySelector('.project-item[data-key="alpha"]')!.classList.contains('active')).toBe(false)
  })

  it('clicking gamma sets active on gamma', () => {
    ;(document.querySelector('.project-item[data-key="beta"]') as HTMLElement).click()
    ;(document.querySelector('.project-item[data-key="gamma"]') as HTMLElement).click()
    expect(document.querySelector('.project-item[data-key="gamma"]')!.classList.contains('active')).toBe(true)
  })

  it('sidebar is not open initially', () => {
    const sidebar = document.getElementById('sidebar') as HTMLElement
    expect(sidebar.classList.contains('open')).toBe(false)
  })

  it('hamburger button toggles sidebar open class', () => {
    ;(document.getElementById('hamburger-btn') as HTMLButtonElement).click()
    const sidebar = document.getElementById('sidebar') as HTMLElement
    expect(sidebar.classList.contains('open')).toBe(true)
  })

  it('clicking hamburger twice closes sidebar', () => {
    const btn = document.getElementById('hamburger-btn') as HTMLButtonElement
    btn.click()
    btn.click()
    const sidebar = document.getElementById('sidebar') as HTMLElement
    expect(sidebar.classList.contains('open')).toBe(false)
  })

  it('project-list has three items', () => {
    expect(document.querySelectorAll('.project-item').length).toBe(3)
  })

  it('each project item has a data-key attribute', () => {
    const items = document.querySelectorAll('.project-item')
    items.forEach(item => {
      expect((item as HTMLElement).dataset.key).toBeTruthy()
    })
  })
})
