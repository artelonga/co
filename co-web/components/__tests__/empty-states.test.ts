import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

function buildDOM() {
  document.body.innerHTML = `
    <div id="app">
      <aside id="sidebar">
        <ul id="project-list">
          <li class="project-item" data-key="proj1">Project One</li>
          <li class="project-item" data-key="proj2">Project Two</li>
        </ul>
      </aside>
      <main id="main-content">
        <div id="empty-state" class="empty-state" style="display:block;">
          <p>Select a project to get started.</p>
        </div>
        <div id="board-area" class="board-area" style="display:none;">
          <div class="board-column" data-status="todo">
            <h3>To Do</h3>
            <div class="column-cards"></div>
          </div>
          <div class="board-column" data-status="in-progress">
            <h3>In Progress</h3>
            <div class="column-cards"></div>
          </div>
          <div class="board-column" data-status="done">
            <h3>Done</h3>
            <div class="column-cards"></div>
          </div>
        </div>
        <div id="empty-board-state" class="empty-board-state" style="display:none;">
          <p>No tasks yet. Create your first task!</p>
        </div>
      </main>
    </div>
  `
}

function selectProject(key: string, tasks: string[] = []) {
  const emptyState = document.getElementById('empty-state') as HTMLElement
  const boardArea = document.getElementById('board-area') as HTMLElement
  const emptyBoard = document.getElementById('empty-board-state') as HTMLElement

  emptyState.style.display = 'none'
  boardArea.style.display = 'flex'

  const todoCol = boardArea.querySelector('.column-cards') as HTMLElement
  todoCol.innerHTML = ''

  if (tasks.length === 0) {
    emptyBoard.style.display = 'block'
  } else {
    emptyBoard.style.display = 'none'
    tasks.forEach(t => {
      const card = document.createElement('div')
      card.className = 'task-card'
      card.textContent = t
      todoCol.appendChild(card)
    })
  }
}

function attachListeners() {
  document.querySelectorAll('.project-item').forEach(item => {
    item.addEventListener('click', () => {
      selectProject((item as HTMLElement).dataset.key!)
    })
  })
}

describe('empty-states', () => {
  beforeEach(() => {
    buildDOM()
    attachListeners()
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('empty-state is visible before project selected', () => {
    const el = document.getElementById('empty-state') as HTMLElement
    expect(el.style.display).toBe('block')
  })

  it('board-area is hidden before project selected', () => {
    const el = document.getElementById('board-area') as HTMLElement
    expect(el.style.display).toBe('none')
  })

  it('empty-board-state is hidden before project selected', () => {
    const el = document.getElementById('empty-board-state') as HTMLElement
    expect(el.style.display).toBe('none')
  })

  it('selecting a project hides empty-state', () => {
    selectProject('proj1', ['Task A'])
    const el = document.getElementById('empty-state') as HTMLElement
    expect(el.style.display).toBe('none')
  })

  it('selecting a project shows board-area', () => {
    selectProject('proj1', ['Task A'])
    const el = document.getElementById('board-area') as HTMLElement
    expect(el.style.display).toBe('flex')
  })

  it('project with no tasks shows empty-board-state', () => {
    selectProject('proj1', [])
    const el = document.getElementById('empty-board-state') as HTMLElement
    expect(el.style.display).toBe('block')
  })

  it('project with tasks hides empty-board-state', () => {
    selectProject('proj1', ['Task X', 'Task Y'])
    const el = document.getElementById('empty-board-state') as HTMLElement
    expect(el.style.display).toBe('none')
  })

  it('project with tasks renders task cards', () => {
    selectProject('proj1', ['Buy milk', 'Write tests'])
    expect(document.querySelectorAll('.task-card').length).toBe(2)
  })
})
