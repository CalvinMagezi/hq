import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToString } from 'react-dom/server'
import { WebSocketProvider } from '~/context/WebSocketContext'
import type { Initiative, TaskItem } from '~/lib/tasksApi'
import { t, tn } from '~/strings'
import { ToastView } from '../ui/Toast'
import { TaskCard } from './TaskCard'
import { TaskDetailMain } from './TaskDetailDrawer'
import { TaskListView } from './TaskListView'
import { dayOfWeek } from './dates'
import { monthRange } from './gantt'

const task = (n: number, over: Partial<TaskItem> = {}) =>
  ({
    id: `tk-${n}`,
    display_id: `HQ-${n}`,
    initiative_id: 'init-1',
    title: `Task ${n}`,
    description: '',
    status: 'to_do',
    priority: null,
    due_date: null,
    start_date: null,
    parent_task_id: null,
    assignees: [],
    tags: [],
    depends_on: [],
    blocked_by: [],
    subtask_count: 0,
    subtask_done: 0,
    blocked_reason: null,
    long_horizon: false,
    archived_at: null,
    created_at: '2026-10-01 08:00:00',
    updated_at: '2026-10-01 08:00:00',
    ...over,
  }) as unknown as TaskItem

const noop = () => {}
const asyncNoop = async () => {}

const drawer = (t1: TaskItem) =>
  renderToString(
    createElement(WebSocketProvider, null, createElement(TaskDetailMain, {
      task: t1,
      allTasks: [t1],
      onClose: noop,
      onUpdate: asyncNoop,
      onRestore: asyncNoop,
      onDelete: asyncNoop,
      onSelectTask: noop,
      onCreateSubtask: asyncNoop,
      busy: false,
    })),
  )

describe('tasks, the leftovers of waves 0 to 4', () => {
  test('Month is five weeks from this week\'s Monday, whatever day it is', () => {
    for (const today of [20_000, 20_001, 20_002, 20_003, 20_004, 20_005, 20_006]) {
      const { start, end } = monthRange(today)
      expect(dayOfWeek(start)).toBe(1)
      expect(end - start + 1).toBe(35)
      expect(today).toBeGreaterThanOrEqual(start)
      expect(today).toBeLessThanOrEqual(start + 6)
    }
  })

  test('list cards are links to the task', () => {
    const initiative = { id: 'init-1', name: 'Launch' } as Initiative
    const tasks = [task(1), task(2)]
    const html = renderToString(
      createElement(TaskListView, {
        tasks,
        allById: new Map(tasks.map((x) => [x.id, x])),
        initiativeById: new Map([[initiative.id, initiative]]),
        onSelect: noop,
        sections: { collapsed: new Set<string>(), onToggle: noop, onSetMany: noop },
      }),
    )
    expect(html).toContain('<a href="/tasks/tk-1"')
    expect(html).toContain('<a href="/tasks/tk-2"')
  })

  test('the check box of a card is the design a-chk, a hit target of its own', () => {
    const html = renderToString(createElement(TaskCard, { task: task(1), select: { ticked: false, onToggle: noop } }))
    expect(html).toMatch(/class="a-chk[^"]*"[^>]*data-hit="select"|data-hit="select"[^>]*class="a-chk/)
  })

  test('check 3: Delete for good shows only on an archived task, and the stale label is plain', () => {
    const active = drawer(task(1))
    expect(active).not.toContain(t('tasks_restore'))
    const archived = drawer(task(2, { archived_at: '2026-10-02 08:00:00' }))
    expect(archived).toContain(t('tasks_restore'))
    expect(archived.length).not.toBe(active.length)
    expect(active).not.toContain('>stale<')
  })

  test('B8: one task done says "Done: {title}", many say "{n} tasks done", both with Undo', () => {
    const one = renderToString(
      createElement(ToastView, { toast: { id: 1, text: t('undo_done_task', { title: 'Renew the domain' }), kind: 'status', undo: asyncNoop, at: Date.now(), late: false }, wide: true, bottom: 24, onUndo: noop, onClose: noop }),
    )
    expect(one).toContain('Done: Renew the domain')
    expect(one).toContain(t('undo'))
    const many = renderToString(
      createElement(ToastView, { toast: { id: 2, text: t('bulk_done_line', { n: 3 }), kind: 'status', undo: asyncNoop, at: Date.now(), late: false }, wide: true, bottom: 24, onUndo: noop, onClose: noop }),
    )
    expect(many).toContain('3 tasks done')
    expect(many).toContain(t('undo'))
    expect(t('bulk_done_title', { n: 3 })).toBe('Mark 3 tasks done?')
    expect(t('bulk_done_keep')).toBe('Keep them as they are')
  })

  test('sub-task counts and the bulk bar speak from the catalogue', () => {
    expect(tn('tasks_sub_task_count', 1)).toBe('1 sub-task')
    expect(tn('tasks_sub_task_count', 3)).toBe('3 sub-tasks')
    expect(t('tasks_bulk_selected', { n: 2 })).toBe('2 selected')
  })
})
