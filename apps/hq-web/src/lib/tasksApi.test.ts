import { expect, test } from 'bun:test'
import { collectTaskPages, type TaskItem } from './tasksApi'

const task = (n: number) => ({ id: `tk-${n}` }) as TaskItem

test('a list longer than one page is read to the end', async () => {
  const all = Array.from({ length: 5 }, (_, n) => task(n))
  const offsets: number[] = []
  const result = await collectTaskPages(async (offset) => {
    offsets.push(offset)
    const tasks = all.slice(offset, offset + 2)
    return { tasks, total: all.length, has_more: offset + tasks.length < all.length }
  })
  expect(offsets).toEqual([0, 2, 4])
  expect(result.tasks.map((t) => t.id)).toEqual(['tk-0', 'tk-1', 'tk-2', 'tk-3', 'tk-4'])
  expect(result.total).toBe(5)
})

test('an empty page ends the loop even if the server still says has_more', async () => {
  let calls = 0
  const result = await collectTaskPages(async () => {
    calls += 1
    return { tasks: [], total: 9, has_more: true }
  })
  expect(calls).toBe(1)
  expect(result.count).toBe(0)
})

test('a row that shifts across a page boundary is kept once', async () => {
  const pages = [
    { tasks: [task(0), task(1)], total: 3, has_more: true },
    { tasks: [task(1), task(2)], total: 3, has_more: false },
  ]
  const offsets: number[] = []
  const result = await collectTaskPages(async (offset) => {
    offsets.push(offset)
    return pages[offsets.length - 1]
  })
  expect(offsets).toEqual([0, 2])
  expect(result.tasks.map((t) => t.id)).toEqual(['tk-0', 'tk-1', 'tk-2'])
  expect(result.count).toBe(3)
})
