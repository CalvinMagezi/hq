import { describe, expect, test } from 'bun:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { renderToString } from 'react-dom/server'
import { createElement } from 'react'
import { doneQuestion } from './doneAsk'
import { awayLinkLine, awaySections } from './tasksAway'
import { chipStatus, matchesTab } from './tasksFilters'
import type { TaskItem } from './tasksApi'
import type { Briefing } from './verificationApi'
import { TasksBody } from '~/components/tasks/TasksBody'
import { boardKey, DONE_SHOWN, moveTargets, visibleDone } from '~/components/tasks/boardMoves'
import { ZOOM_KEY, isZoom } from '~/components/tasks/gantt'
import { sweepProgress } from '~/components/tasks/verification/verification'
import { createOutbox, type OutboxEntry, type OutboxStore } from './outbox'

const mk = (id: string, over: Partial<TaskItem> = {}) =>
  ({ id, status: 'in_progress', depends_on: [], archived_at: null, updated_at: '2026-01-01 00:00:00', ...over }) as unknown as TaskItem

const briefing = (over: Partial<Briefing> = {}): Briefing =>
  ({
    since: '', mode: 'enforce', needs_you: [], needs_you_total: 0, bulk_offer: null, completed: [], completed_total: 0,
    reworked: [], reworked_total: 0, veto_windows: [], audit_pool: 0, open_audits: 0, health: {}, ...over,
  }) as unknown as Briefing

const src = (dir: string): string[] =>
  readdirSync(dir).flatMap((n) => {
    const p = join(dir, n)
    if (statSync(p).isDirectory()) return src(p)
    return /\.tsx?$/.test(n) && !/\.test\./.test(n) ? [p] : []
  })

describe('tasks finish', () => {
  test('check 1: the list renders before the checking blocks', () => {
    const html = renderToString(createElement(TasksBody, { list: createElement('p', null, 'LIST'), checking: createElement('p', null, 'CHECKING') }))
    expect(html.indexOf('LIST')).toBeGreaterThan(-1)
    expect(html.indexOf('LIST')).toBeLessThan(html.indexOf('CHECKING'))
  })

  test('check 2 and 8: a done with a pending check asks first, then the dependents question', () => {
    const a = mk('a')
    const dep = mk('b', { depends_on: ['a'] })
    expect(doneQuestion(a, { status: 'complete' }, [a], true)).toBe('check')
    expect(doneQuestion(a, { status: 'complete' }, [a, dep], true)).toBe('check')
    expect(doneQuestion(a, { status: 'complete' }, [a, dep], false)).toBe('dependents')
    expect(doneQuestion(a, { status: 'complete' }, [a], false)).toBeNull()
    expect(doneQuestion(a, { title: 'x' }, [a], true)).toBeNull()
  })

  test('check 3: delete for good is offered only for an archived task', () => {
    const drawer = readFileSync('src/components/tasks/TaskDetailDrawer.tsx', 'utf8')
    expect(drawer).toContain('{!archived && (')
    expect(drawer).toContain('onDelete(task.id, true, true)')
    expect(drawer).toContain('onDelete(task.id, task.subtask_count > 0)')
  })

  test('check 4: none of the retired words are left in the Tasks components', () => {
    const banned = [/Verify now/, /'Veto'|>Veto</, /Return to to_do/, /agree or not/, /Second opinion/, /Complete anyway/, /\btier \d/i, /Inspected/, /Artifact:/, /Inconclusive/]
    const files = src('src/components/tasks').filter((f) => !f.endsWith('.d.ts'))
    const hits = files.flatMap((f) => {
      const text = readFileSync(f, 'utf8')
        .split('\n')
        .filter((l) => !/^\s*(\/\/|\*|\/\*)/.test(l) && /['"`>][^'"`<>]*[A-Za-z]/.test(l) && !/\b(import|type|interface|kind|tier:|artifact:)\b/.test(l))
        .join('\n')
      return banned.filter((b) => b.test(text)).map((b) => `${f}: ${b}`)
    })
    expect(hits).toEqual([])
  })

  test('check 5: board cards are links, Move is a button, M and the arrows are keys', () => {
    const board = readFileSync('src/components/tasks/TaskBoardView.tsx', 'utf8')
    expect(board).toContain('a-card')
    expect(board).toContain('data-board-move')
    expect(boardKey('m')).toBe('move')
    expect(boardKey('M')).toBe('move')
    expect(boardKey('ArrowLeft')).toBe('left')
    expect(boardKey('x')).toBeNull()
    expect(moveTargets({ status: 'to_do', first_ready_for_review_at: null })).not.toContain('ready_for_review')
    expect(moveTargets({ status: 'to_do', first_ready_for_review_at: 'x' })).toContain('ready_for_review')
    const done = Array.from({ length: 9 }, (_, i) => mk(`d${i}`, { completed_at: `2026-01-0${i + 1}` } as Partial<TaskItem>))
    const shown = visibleDone(done, false)
    expect(shown.length).toBe(DONE_SHOWN)
    expect(shown[0].id).toBe('d8')
    expect(visibleDone(done, true).length).toBe(9)
  })

  test('check 6: Month shows Mondays only and the zoom is remembered', () => {
    const view = readFileSync('src/components/tasks/TaskGanttView.tsx', 'utf8')
    expect(view).toContain('usePersistedState<Zoom>(ZOOM_KEY')
    expect(view).toContain("t('timeline_monday'")
    expect(view).toContain("zoom === 'month' ? []")
    expect(isZoom('month')).toBe(true)
    expect(isZoom('year')).toBe(false)
    expect(ZOOM_KEY).toContain('zoom')
  })

  test('check 7: a task made offline waits in the outbox and is sent once by its client id', async () => {
    let saved: OutboxEntry[] = []
    const store: OutboxStore = { load: async () => saved.slice(), save: async (_h, e) => void (saved = e) }
    const box = createOutbox(store)
    const entry: OutboxEntry = { client_id: 'c1', kind: 'task', hq_id: 'h', payload: { title: 'Pay rent' }, created_at: 1, attempts: 0 }
    await box.add(entry)
    await box.add({ ...entry, client_id: 'm1', kind: 'message' })
    const sent: string[] = []
    await box.drain('h', async (e) => (sent.push(e.client_id), 'sent'), undefined, 'task')
    expect(sent).toEqual(['c1'])
    expect(saved.map((e) => e.client_id)).toEqual(['m1'])
    const page = readFileSync('src/routes/tasks.tsx', 'utf8')
    expect(page).toContain('client_id: newClientId()')
  })

  test('check 9: the away page lists finishing, finished, went back, waiting in that order', () => {
    const b = briefing({
      completed_total: 3,
      completed: [{ initiative_id: 'i', initiative: 'Home', count: 3, by_level: {}, tier2: 0 }] as Briefing['completed'],
      veto_windows: [{ task_id: 't1', display_id: 'W-1', title: 'One', release_at: 100 }] as Briefing['veto_windows'],
      reworked_total: 1,
      reworked: [{ task_id: 't2', display_id: 'W-2', title: 'Two', returns: 1, last_finding: 'x', status: 'to_do' }] as Briefing['reworked'],
      needs_you_total: 1,
      needs_you: [{ task_id: 't3', display_id: 'W-3', title: 'Three', reason: 'r' }] as Briefing['needs_you'],
      open_audits: 2,
    })
    expect(awaySections(b, 0, true).map((s) => s.id)).toEqual(['finishing', 'finished', 'back', 'waiting', 'spot'])
    expect(awaySections(b, 0, false).map((s) => s.id)).not.toContain('spot')
    expect(awayLinkLine(b)).toContain('3')
    expect(awayLinkLine(briefing())).toBeNull()
    expect(readFileSync('src/components/tasks/AwayPage.tsx', 'utf8')).toContain('data-away-section')
    expect(sweepProgress('running', { total: 10, buckets: {}, states: { planned: 3, verdict: 7 } })?.label).toBe('7 of 10 done')
  })

  test('tabs and chips narrow the list', () => {
    expect(chipStatus('stuck')).toBe('blocked')
    expect(chipStatus('active')).toBeNull()
    const today = '2026-05-10'
    expect(matchesTab(mk('a', { due_date: '2026-05-09' } as Partial<TaskItem>), 'today', today)).toBe(true)
    expect(matchesTab(mk('a', { status: 'to_do', due_date: '2026-06-01' } as Partial<TaskItem>), 'today', today)).toBe(false)
    expect(matchesTab(mk('a', { status: 'to_do', due_date: '2026-06-01' } as Partial<TaskItem>), 'upcoming', today)).toBe(true)
    expect(matchesTab(mk('a', { status: 'complete' }), 'all', today)).toBe(true)
  })
})
