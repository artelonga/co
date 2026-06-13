import { describe, it, expect, beforeEach, afterEach } from 'vitest'

// @vitest-environment happy-dom

// CO-128: exercise the real SPA modules (not reconstructed stand-ins).
import { lineDiff } from '../../static/variants/a/modules/sync/conflict-diff.js'
import { buildConflictModal } from '../../static/variants/a/modules/sync/conflict-modal.js'
import {
  ConflictRound,
  localVariantPath,
} from '../../static/variants/a/modules/sync/conflict-round.js'

function samplePayload(over: Record<string, unknown> = {}) {
  return {
    universe_key: 'u1',
    path: 'notes/x.md',
    kind: 'both_modified',
    local: { body: 'line one\nLOCAL change\nline three', body_hash: 'aaa' },
    remote: { body: 'line one\nREMOTE change\nline three', body_hash: 'bbb' },
    base: { body_hash: 'base0' },
    ...over,
  }
}

describe('conflict-diff: lineDiff', () => {
  it('marks equal, deleted and added lines', () => {
    const { rows, added, removed } = lineDiff('a\nb\nc', 'a\nX\nc')
    expect(rows[0]).toMatchObject({ kind: 'equal', left: 'a', right: 'a' })
    expect(rows.some((r) => r.kind === 'del' && r.left === 'b')).toBe(true)
    expect(rows.some((r) => r.kind === 'add' && r.right === 'X')).toBe(true)
    expect(added).toBe(1)
    expect(removed).toBe(1)
  })

  it('identical input yields all-equal rows', () => {
    const { rows, added, removed } = lineDiff('same\ntext', 'same\ntext')
    expect(added).toBe(0)
    expect(removed).toBe(0)
    expect(rows.every((r) => r.kind === 'equal')).toBe(true)
  })
})

describe('conflict-modal: buildConflictModal', () => {
  afterEach(() => {
    document.body.replaceChildren()
  })

  it('renders both versions side by side from the ConflictPayload', () => {
    const { overlay } = buildConflictModal(samplePayload(), {})
    document.body.appendChild(overlay)
    const text = overlay.textContent || ''
    expect(text).toContain('LOCAL change')
    expect(text).toContain('REMOTE change')
    // Two header columns: local + remote.
    expect(overlay.querySelectorAll('th').length).toBe(2)
    // The entry path is surfaced (Finder-style subtitle).
    expect(text).toContain('notes/x.md')
  })

  it('renders the three action buttons (Keep both / Ignore / Replace)', () => {
    const { overlay } = buildConflictModal(samplePayload(), {})
    document.body.appendChild(overlay)
    const btns = overlay.querySelectorAll('.co-conflict-btn')
    expect(btns.length).toBe(3)
  })

  it('clicking Replace resolves with action=replace', () => {
    let resolved: { action: string; applyToAll: boolean } | null = null
    const { overlay } = buildConflictModal(samplePayload(), {
      onResolve: (action: string, applyToAll: boolean) => {
        resolved = { action, applyToAll }
      },
    })
    document.body.appendChild(overlay)
    overlay.querySelector<HTMLButtonElement>('.co-conflict-btn-primary')!.click()
    expect(resolved).toEqual({ action: 'replace', applyToAll: false })
  })

  it('reports apply-to-all when the checkbox is ticked', () => {
    let resolved: { action: string; applyToAll: boolean } | null = null
    const { overlay } = buildConflictModal(samplePayload(), {
      onResolve: (action: string, applyToAll: boolean) => {
        resolved = { action, applyToAll }
      },
    })
    document.body.appendChild(overlay)
    overlay.querySelector<HTMLInputElement>('.co-conflict-applyall-box')!.checked = true
    overlay.querySelector<HTMLButtonElement>('.co-conflict-btn-primary')!.click()
    expect(resolved).toEqual({ action: 'replace', applyToAll: true })
  })

  it('keyboard shortcuts: 1=ignore, 2=replace, 3=keep both, Esc=cancel', () => {
    const press = (key: string): string => {
      let got = ''
      const { overlay, destroy } = buildConflictModal(samplePayload(), {
        onResolve: (action: string) => {
          got = action
        },
      })
      document.body.appendChild(overlay)
      document.dispatchEvent(new KeyboardEvent('keydown', { key }))
      destroy()
      return got
    }
    expect(press('1')).toBe('ignore')
    expect(press('2')).toBe('replace')
    expect(press('3')).toBe('keep_both')
    expect(press('Escape')).toBe('cancel')
  })

  it('resolves only once (button after keyboard is a no-op)', () => {
    const calls: string[] = []
    const { overlay } = buildConflictModal(samplePayload(), {
      onResolve: (action: string) => calls.push(action),
    })
    document.body.appendChild(overlay)
    document.dispatchEvent(new KeyboardEvent('keydown', { key: '1' }))
    overlay.querySelector<HTMLButtonElement>('.co-conflict-btn-primary')!.click()
    expect(calls).toEqual(['ignore'])
  })
})

describe('conflict-round: ConflictRound', () => {
  it('localVariantPath inserts .local before the extension', () => {
    expect(localVariantPath('notes/x.md')).toBe('notes/x.local.md')
    expect(localVariantPath('readme')).toBe('readme.local')
  })

  it('apply-to-all batches the choice across the rest of the round', async () => {
    const opened: string[] = []
    const api = {
      ignore: async (c: any) => ({ ok: true, action: 'ignore', path: c.path }),
      replace: async (c: any) => ({ ok: true, action: 'replace', path: c.path }),
      keepBoth: async (c: any) => ({ ok: true, action: 'keep_both', path: c.path }),
    }
    // Modal answers only the FIRST conflict; the rest must be auto-applied.
    const openModal = async (payload: any) => {
      opened.push(payload.path)
      return { action: 'replace', applyToAll: true }
    }
    const round = new ConflictRound({ api, openModal })
    round.enqueue(samplePayload({ path: 'a.md' }))
    round.enqueue(samplePayload({ path: 'b.md' }))
    round.enqueue(samplePayload({ path: 'c.md' }))

    const { results, cancelled } = await round.run()
    expect(cancelled).toBe(false)
    expect(opened).toEqual(['a.md']) // modal shown once only
    expect(results.map((r: any) => r.action)).toEqual(['replace', 'replace', 'replace'])
    expect(round.applyToAllActive).toBe(true)
  })

  it('asks per-conflict when apply-to-all stays unchecked', async () => {
    const opened: string[] = []
    const api = {
      ignore: async () => ({ ok: true }),
      replace: async () => ({ ok: true }),
      keepBoth: async () => ({ ok: true }),
    }
    const openModal = async (payload: any) => {
      opened.push(payload.path)
      return { action: 'ignore', applyToAll: false }
    }
    const round = new ConflictRound({ api, openModal })
    round.enqueue(samplePayload({ path: 'a.md' }))
    round.enqueue(samplePayload({ path: 'b.md' }))
    await round.run()
    expect(opened).toEqual(['a.md', 'b.md']) // shown for every conflict
  })

  it('Esc cancels the whole round and stops processing', async () => {
    const api = {
      ignore: async () => ({ ok: true }),
      replace: async () => ({ ok: true }),
      keepBoth: async () => ({ ok: true }),
    }
    const openModal = async () => ({ action: 'cancel', applyToAll: false })
    const round = new ConflictRound({ api, openModal })
    round.enqueue(samplePayload({ path: 'a.md' }))
    round.enqueue(samplePayload({ path: 'b.md' }))
    const { results, cancelled } = await round.run()
    expect(cancelled).toBe(true)
    expect(results.length).toBe(0)
  })
})
