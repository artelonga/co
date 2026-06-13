import { describe, it, expect } from 'vitest'

// @vitest-environment happy-dom

// CO-396: the JS mirror of game_core::time_layout's project-timeline engine.
// These assertions parallel the Rust `project_timeline_tests` so the two
// implementations cannot silently diverge.
// @ts-expect-error — plain JS ES module, no type declarations.
import { layoutProjectTimeline, ganttSpan, entryToTimelineEntry } from '../../static/shared/lib/co-time.js'

const DAY = 86_400_000

function task(path: string, fm: Record<string, unknown> = {}, entry_type = 'task') {
  return { path, title: path, entry_type, frontmatter: fm }
}

describe('ganttSpan duration semantics', () => {
  it('prefers scheduled→due when both present', () => {
    const te = entryToTimelineEntry(task('a', {
      scheduled_at: '2026-01-11', due_at: '2026-01-15',
    }))
    const span = ganttSpan(te)
    expect(span.end_ms - span.start_ms).toBe(4 * DAY)
  })

  it('falls back to created→done', () => {
    const te = entryToTimelineEntry(task('a', { status: 'done' }))
    te.created_ms = 2 * DAY
    te.done_ms = 9 * DAY
    const span = ganttSpan(te)
    expect(span.start_ms).toBe(2 * DAY)
    expect(span.end_ms).toBe(9 * DAY)
  })

  it('degrades a single date to a point', () => {
    const te = entryToTimelineEntry(task('a', { due_at: '2026-03-01' }))
    const span = ganttSpan(te)
    expect(span.start_ms).toBe(span.end_ms)
  })

  it('created-only has no span (→ backlog)', () => {
    const te = entryToTimelineEntry(task('a', { created: '2026-01-01' }))
    expect(ganttSpan(te)).toBeNull()
  })
})

describe('layoutProjectTimeline', () => {
  it('groups dated entries into lanes by the chosen field', () => {
    const tl = layoutProjectTimeline([
      task('a', { epic: 'auth', due_at: '2026-01-02' }),
      task('b', { epic: 'billing', due_at: '2026-01-03' }),
      task('c', { epic: 'auth', due_at: '2026-01-04' }),
    ], 'epic')
    expect(tl.lanes.length).toBe(2)
    expect(tl.lanes[0].key).toBe('auth')
    expect(tl.lanes[0].items.map((i: { id: string }) => i.id)).toEqual(['a', 'c'])
  })

  it('parks undated tasks in the backlog (nothing disappears)', () => {
    const tl = layoutProjectTimeline([
      task('dated', { epic: 'x', due_at: '2026-02-01' }),
      task('bare', { epic: 'x' }),
    ], 'epic')
    expect(tl.lanes[0].items.length).toBe(1)
    expect(tl.backlog.map((b: { id: string }) => b.id)).toEqual(['bare'])
  })

  it('lifts release/milestone entries into markers', () => {
    const tl = layoutProjectTimeline([
      task('v1', { event_at: '2026-06-01' }, 'release'),
      task('beta', { due_at: '2026-03-01' }, 'milestone'),
      task('t', { status: 'doing', due_at: '2026-01-01' }),
    ], 'status')
    expect(tl.markers.length).toBe(2)
    expect(tl.markers[0].kind).toBe('release')
    expect(tl.lanes.length).toBe(1)
    expect(tl.lanes[0].items[0].id).toBe('t')
  })

  it('defaults unknown grouping to status', () => {
    const tl = layoutProjectTimeline([task('a', { status: 'doing', due_at: '2026-01-01' })], 'bogus')
    expect(tl.group_by).toBe('status')
  })
})
