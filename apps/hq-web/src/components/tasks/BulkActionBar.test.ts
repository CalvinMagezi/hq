import { expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToString } from 'react-dom/server'
import { BulkActionBar } from './BulkActionBar'

const noop = () => {}
const render = (props: Partial<Parameters<typeof BulkActionBar>[0]> = {}) =>
  renderToString(
    createElement(BulkActionBar, {
      total: 12,
      ticked: 3,
      progress: null,
      message: null,
      onSelectAll: noop,
      onClear: noop,
      onDone: noop,
      onStatus: noop,
      onPriority: noop,
      archiveLabel: 'Archive',
      onArchive: noop,
      ...props,
    })
  )

test('it counts what is ticked and offers to take the whole list', () => {
  const html = render()
  expect(html).toContain('3 selected')
  expect(html).toContain('Select all 12')
})

test('with everything ticked the same button clears instead', () => {
  const html = render({ ticked: 12 })
  expect(html).toContain('Clear')
  expect(html).not.toContain('Select all')
})

test('with nothing ticked the actions are off', () => {
  const html = render({ ticked: 0 })
  expect(html).toMatch(/<button[^>]*disabled[^>]*>Set status/)
  expect(html).toMatch(/<button[^>]*disabled[^>]*>Set priority/)
})

test('while it runs it shows progress and locks every control', () => {
  const html = render({ progress: { settled: 4, total: 9 } })
  expect(html).toContain('4')
  expect(html).toContain('of')
  expect(html).not.toContain('3 selected')
  expect(html).toMatch(/<button[^>]*disabled[^>]*>Done/)
})

test('the last result shows once the run is over', () => {
  expect(render({ message: '12 tasks moved to Complete' })).toContain('12 tasks moved to Complete')
})

const archiveButton = (html: string) => html.match(/<button[^>]*>(Archive|Unarchive)<\/button>/)

test('it archives the ticked tasks, or unarchives them when the archive is open', () => {
  expect(archiveButton(render())?.[1]).toBe('Archive')
  expect(archiveButton(render({ archiveLabel: 'Unarchive' }))?.[1]).toBe('Unarchive')
})

// The class list always names the `disabled:` variants, so look for the attribute itself.
const isDisabled = (button: string | undefined) => /\sdisabled(=|>|\s)/.test(button ?? '')

test('the archive button waits for something to be ticked', () => {
  expect(isDisabled(archiveButton(render({ ticked: 3 }))?.[0])).toBe(false)
  expect(isDisabled(archiveButton(render({ ticked: 0 }))?.[0])).toBe(true)
})
