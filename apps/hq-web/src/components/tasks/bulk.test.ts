import { expect, test } from 'bun:test'
import { HqHttpError } from '~/lib/hqAuth'
import { HTTP_CONFLICT, type TaskItem } from '~/lib/tasksApi'
import { bulkMessage, failureNotice, failureReason, runBulk, splitUnchanged, uniqueWarnings } from './bulk'
import { keepVisible, withIds } from './useTaskSelection'

const task = (n: number, overrides: Partial<TaskItem> = {}) =>
  ({ id: `tk-${n}`, display_id: `HQ-${n}`, status: 'ready_for_review', priority: null, ...overrides }) as TaskItem

const tasks = (count: number) => Array.from({ length: count }, (_, n) => task(n + 1))

test('every task is run once and the results come back with their task', async () => {
  const out = await runBulk(tasks(5), async (t) => t.id.toUpperCase())
  expect(out.failed).toEqual([])
  expect(out.done.map((d) => d.value).sort()).toEqual(['TK-1', 'TK-2', 'TK-3', 'TK-4', 'TK-5'])
  for (const d of out.done) expect(d.value).toBe(d.task.id.toUpperCase())
})

test('no more than the limit run at once, and the limit is reached', async () => {
  let inFlight = 0
  let peak = 0
  const out = await runBulk(
    tasks(12),
    async () => {
      inFlight += 1
      peak = Math.max(peak, inFlight)
      await new Promise((r) => setTimeout(r, 2))
      inFlight -= 1
    },
    undefined,
    3
  )
  expect(out.done).toHaveLength(12)
  expect(peak).toBe(3)
})

test('one task failing does not stop the others', async () => {
  const out = await runBulk(tasks(6), async (t) => {
    if (t.id === 'tk-3') throw new Error('nope')
    return t.id
  })
  expect(out.done).toHaveLength(5)
  expect(out.failed.map((f) => f.task.id)).toEqual(['tk-3'])
  expect((out.failed[0].error as Error).message).toBe('nope')
})

test('progress is reported once per settled task, failures included', async () => {
  const seen: [number, number][] = []
  await runBulk(
    tasks(4),
    async (t) => {
      if (t.id === 'tk-2') throw new Error('x')
    },
    (settled, total) => seen.push([settled, total]),
    1
  )
  expect(seen).toEqual([
    [1, 4],
    [2, 4],
    [3, 4],
    [4, 4],
  ])
})

test('an empty selection runs nothing', async () => {
  let calls = 0
  const out = await runBulk([], async () => void (calls += 1))
  expect(calls).toBe(0)
  expect(out).toEqual({ done: [], failed: [] })
})

test('a 409 reads as someone else changing the task, anything else keeps its own message', () => {
  expect(failureReason(new HqHttpError('claim conflict', HTTP_CONFLICT))).toBe('was changed by someone else')
  expect(failureReason(new HqHttpError('unknown status', 400))).toBe('unknown status')
  expect(failureReason('weird')).toBe('could not be updated')
})

test('the failure notice names the first few and counts the rest', () => {
  expect(failureNotice([])).toBeNull()
  const failed = (n: number) => ({ task: task(n), error: new HqHttpError('c', HTTP_CONFLICT) })
  expect(failureNotice([failed(1)])).toBe('1 could not be updated: HQ-1 was changed by someone else.')
  expect(failureNotice([failed(1), failed(2), failed(3), failed(4), failed(5)])).toBe(
    '5 could not be updated: HQ-1 was changed by someone else; HQ-2 was changed by someone else; HQ-3 was changed by someone else; and 2 more.'
  )
})

test('the result line says what happened', () => {
  expect(bulkMessage(12, 12, 0, 'moved to Complete')).toBe('12 tasks moved to Complete')
  expect(bulkMessage(1, 1, 0, 'moved to Complete')).toBe('1 task moved to Complete')
  expect(bulkMessage(10, 12, 0, 'moved to Complete')).toBe('10 of 12 tasks moved to Complete')
  expect(bulkMessage(0, 12, 0, 'moved to Complete')).toBe('0 of 12 tasks moved to Complete')
  expect(bulkMessage(3, 3, 2, 'set to high priority')).toBe('3 tasks set to high priority, 2 already there')
  expect(bulkMessage(0, 0, 4, 'moved to Complete')).toBe('Nothing to change, 4 tasks already there')
  expect(bulkMessage(0, 0, 1, 'moved to Complete')).toBe('Nothing to change, 1 task already there')
})

test('tasks already at the target are left alone and counted', () => {
  const mixed = [task(1, { status: 'complete' }), task(2), task(3, { status: 'complete' }), task(4)]
  const { toChange, unchanged } = splitUnchanged(mixed, (t) => t.status === 'complete')
  expect(toChange.map((t) => t.id)).toEqual(['tk-2', 'tk-4'])
  expect(unchanged).toBe(2)
})

test('a warning raised by many updates is said once', () => {
  expect(uniqueWarnings([['a'], undefined, ['a', 'b'], []])).toEqual(['a', 'b'])
})

test('ticking and un-ticking leaves the original set alone', () => {
  const start: ReadonlySet<string> = new Set(['tk-1'])
  const grown = withIds(start, ['tk-2', 'tk-3'], true)
  expect([...grown]).toEqual(['tk-1', 'tk-2', 'tk-3'])
  expect([...withIds(grown, ['tk-1', 'tk-9'], false)]).toEqual(['tk-2', 'tk-3'])
  expect([...start]).toEqual(['tk-1'])
})

test('a ticked task that left the list is dropped, and nothing changes when none did', () => {
  const ticked: ReadonlySet<string> = new Set(['tk-1', 'tk-2', 'tk-3'])
  expect([...keepVisible(ticked, [task(1), task(3), task(4)])]).toEqual(['tk-1', 'tk-3'])
  expect(keepVisible(ticked, tasks(3))).toBe(ticked)
})
