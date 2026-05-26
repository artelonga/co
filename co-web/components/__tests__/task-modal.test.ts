import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

function buildDOM() {
  document.body.innerHTML = `
    <div id="modal-overlay" class="modal-overlay">
      <div id="task-modal" class="modal">
        <button id="modal-close" class="modal-close">&times;</button>
        <input id="task-title" type="text" value="" placeholder="Task title" />
        <select id="task-status">
          <option value="todo">To Do</option>
          <option value="in-progress">In Progress</option>
          <option value="done">Done</option>
        </select>
        <textarea id="task-description" placeholder="Description"></textarea>
        <button id="modal-submit" type="button">Save</button>
      </div>
    </div>
  `
}

function openModal(title = '', status = 'todo', description = '') {
  const overlay = document.getElementById('modal-overlay') as HTMLElement
  overlay.classList.add('visible')
  ;(document.getElementById('task-title') as HTMLInputElement).value = title
  ;(document.getElementById('task-status') as HTMLSelectElement).value = status
  ;(document.getElementById('task-description') as HTMLTextAreaElement).value = description
}

function closeModal() {
  const overlay = document.getElementById('modal-overlay') as HTMLElement
  overlay.classList.remove('visible')
}

function attachListeners() {
  document.getElementById('modal-close')!.addEventListener('click', closeModal)
  document.getElementById('modal-submit')!.addEventListener('click', () => {
    const title = (document.getElementById('task-title') as HTMLInputElement).value
    const status = (document.getElementById('task-status') as HTMLSelectElement).value
    const description = (document.getElementById('task-description') as HTMLTextAreaElement).value
    document.dispatchEvent(new CustomEvent('task:save', { detail: { title, status, description } }))
    closeModal()
  })
  document.addEventListener('keydown', (e: KeyboardEvent) => {
    if (e.key === 'Escape') closeModal()
  })
}

describe('task-modal', () => {
  beforeEach(() => {
    buildDOM()
    attachListeners()
  })

  afterEach(() => {
    document.body.innerHTML = ''
  })

  it('modal is not visible initially', () => {
    const overlay = document.getElementById('modal-overlay') as HTMLElement
    expect(overlay.classList.contains('visible')).toBe(false)
  })

  it('openModal adds visible class to overlay', () => {
    openModal()
    expect(document.getElementById('modal-overlay')!.classList.contains('visible')).toBe(true)
  })

  it('modal opens with correct title', () => {
    openModal('Fix bug')
    expect((document.getElementById('task-title') as HTMLInputElement).value).toBe('Fix bug')
  })

  it('title defaults to empty string', () => {
    openModal()
    expect((document.getElementById('task-title') as HTMLInputElement).value).toBe('')
  })

  it('status select has todo option', () => {
    const select = document.getElementById('task-status') as HTMLSelectElement
    const values = Array.from(select.options).map(o => o.value)
    expect(values).toContain('todo')
  })

  it('status select has in-progress option', () => {
    const select = document.getElementById('task-status') as HTMLSelectElement
    const values = Array.from(select.options).map(o => o.value)
    expect(values).toContain('in-progress')
  })

  it('status select has done option', () => {
    const select = document.getElementById('task-status') as HTMLSelectElement
    const values = Array.from(select.options).map(o => o.value)
    expect(values).toContain('done')
  })

  it('close button removes visible class', () => {
    openModal('Test')
    ;(document.getElementById('modal-close') as HTMLButtonElement).click()
    expect(document.getElementById('modal-overlay')!.classList.contains('visible')).toBe(false)
  })

  it('ESC key closes modal', () => {
    openModal('Test')
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    expect(document.getElementById('modal-overlay')!.classList.contains('visible')).toBe(false)
  })

  it('submit dispatches task:save custom event', () => {
    let received: CustomEvent | null = null
    document.addEventListener('task:save', (e) => { received = e as CustomEvent })
    openModal('My Task', 'in-progress', 'Some work')
    ;(document.getElementById('modal-submit') as HTMLButtonElement).click()
    expect(received).not.toBeNull()
    expect((received as unknown as CustomEvent).detail.title).toBe('My Task')
  })

  it('submit event includes status', () => {
    let received: CustomEvent | null = null
    document.addEventListener('task:save', (e) => { received = e as CustomEvent })
    openModal('Task', 'done', '')
    ;(document.getElementById('modal-submit') as HTMLButtonElement).click()
    expect((received as unknown as CustomEvent).detail.status).toBe('done')
  })

  it('submit closes the modal', () => {
    openModal('Task')
    ;(document.getElementById('modal-submit') as HTMLButtonElement).click()
    expect(document.getElementById('modal-overlay')!.classList.contains('visible')).toBe(false)
  })
})
