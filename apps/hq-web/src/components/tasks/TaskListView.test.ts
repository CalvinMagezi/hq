import { expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToString } from 'react-dom/server'
import type { Initiative, TaskItem } from '~/lib/tasksApi'
import { TaskListView, type ListSections, type ListSelection } from './TaskListView'

const initiative = { id: 'init-1', name: 'Launch' } as Initiative
const academy = { id: 'init-2', name: 'Academy' } as Initiative

const makeTask = (n: number, overrides: Partial<TaskItem> = {}) =>
  ({
    id: `tk-${n}`,
    display_id: `HQ-${n}`,
    initiative_id: 'init-1',
    title: `Task ${n}`,
    description: '',
    status: 'ready_for_review',
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
    created_at: '2026-10-01 08:00:00',
    updated_at: '2026-10-01 08:00:00',
    ...overrides,
  }) as TaskItem

const noop = () => {}
const unfolded = (collapsed: string[] = []): ListSections => ({
  collapsed: new Set(collapsed),
  onToggle: noop,
  onSetMany: noop,
})

const render = (tasks: TaskItem[], selection?: ListSelection, sections: ListSections = unfolded()) =>
  renderToString(
    createElement(TaskListView, {
      tasks,
      allById: new Map(tasks.map((t) => [t.id, t])),
      initiativeById: new Map([
        [initiative.id, initiative],
        [academy.id, academy],
      ]),
      onSelect: noop,
      sections,
      selection,
    })
  )

const academyTask = (n: number) => makeTask(n, { initiative_id: 'init-2' })


// React writes "1<!-- --> sub-task", so match on the button's text rather than the count.
const SUBTASK_TOGGLE = ' sub-task</button>'

const checkboxes = (html: string) => html.match(/<input[^>]*type="checkbox"[^>]*>/g) ?? []

test('a plain list has no checkboxes', () => {
  expect(checkboxes(render([makeTask(1), makeTask(2)]))).toHaveLength(0)
})

test('picking shows a box on every card and one for the group, ticked where chosen', () => {
  const html = render([makeTask(1), makeTask(2)], { ticked: new Set(['tk-1']), onToggle: noop, onToggleMany: noop })
  const boxes = checkboxes(html)
  expect(boxes).toHaveLength(3)
  expect(boxes.filter((b) => b.includes('checked'))).toHaveLength(1)
  expect(html).toContain('aria-label="Select HQ-1"')
  expect(html).toContain('aria-label="Select all in Launch"')
})

test('picking shows every sub-task and drops the sub-task toggle', () => {
  const parent = makeTask(1, { subtask_count: 1 })
  const child = makeTask(2, { parent_task_id: 'tk-1' })
  const html = render([parent, child], { ticked: new Set(), onToggle: noop, onToggleMany: noop })
  expect(html).toContain('Task 2')
  expect(html).not.toContain(SUBTASK_TOGGLE)
})

test('the plain list keeps its sub-task toggle', () => {
  const parent = makeTask(1, { subtask_count: 1 })
  expect(render([parent, makeTask(2, { parent_task_id: 'tk-1' })])).toContain(SUBTASK_TOGGLE)
})

const headerButton = (html: string, name: string) =>
  html.match(new RegExp(`<button[^>]*aria-expanded="(true|false)"[^>]*>(?:(?!</button>).)*${name}`))?.[1]

test('a section header says whether it is open, and opens by default', () => {
  const html = render([makeTask(1), academyTask(2)])
  expect(headerButton(html, 'Launch')).toBe('true')
  expect(headerButton(html, 'Academy')).toBe('true')
  expect(html).toContain('Task 1')
  expect(html).toContain('Task 2')
})

test('a folded section keeps its header and count but hides its tasks', () => {
  const html = render([makeTask(1), makeTask(2), academyTask(3)], undefined, unfolded(['Launch']))
  expect(headerButton(html, 'Launch')).toBe('false')
  expect(html).toContain('(<!-- -->2<!-- -->)')
  expect(html).not.toContain('Task 1')
  expect(html).not.toContain('Task 2')
  expect(html).toContain('Task 3')
})

test('collapse all and expand all show with several sections and switch off when they would do nothing', () => {
  const tasks = [makeTask(1), academyTask(2)]
  const open = render(tasks)
  expect(open).toMatch(/<button[^>]*>Collapse all/)
  expect(open).toMatch(/<button[^>]*disabled[^>]*>Expand all/)
  const shut = render(tasks, undefined, unfolded(['Launch', 'Academy']))
  expect(shut).toMatch(/<button[^>]*disabled[^>]*>Collapse all/)
  expect(shut).toMatch(/<button[^>]*>Expand all/)
})

test('a single section has nothing to collapse all of', () => {
  expect(render([makeTask(1), makeTask(2)])).not.toContain('Collapse all')
})

test('while picking, a folded section keeps its box but cannot be ticked', () => {
  const html = render(
    [makeTask(1), academyTask(2)],
    { ticked: new Set(), onToggle: noop, onToggleMany: noop },
    unfolded(['Launch'])
  )
  expect(html).toMatch(/<input[^>]*disabled[^>]*aria-label="Select all in Launch"/)
  expect(html).not.toMatch(/<input[^>]*disabled[^>]*aria-label="Select all in Academy"/)
  expect(html).not.toContain('aria-label="Select HQ-1"')
  expect(html).toContain('aria-label="Select HQ-2"')
})
