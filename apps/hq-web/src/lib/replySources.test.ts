import { describe, expect, test } from 'bun:test'
import { chatMadeItems, madeNotes, sourcesOf } from './replySources'

const step = (toolName: string, inputArgs?: Record<string, unknown>, status: 'running' | 'done' = 'done') => ({ toolName, status, inputArgs })

describe('what a reply read', () => {
  test('lists notes, files and pages once each, named for people', () => {
    const list = sourcesOf([
      step('vault_read', { path: 'Notebooks/Shop/Stock rules.md' }),
      step('vault_read_section', { path: 'Notebooks/Shop/Stock rules.md' }),
      step('web_fetch', { url: 'https://www.prices.example.com/quote?id=9' }),
      step('read_file', { path: '/work/Quote A.pdf' }),
    ])
    expect(list.map((s) => [s.kind, s.title])).toEqual([
      ['note', 'Stock rules'],
      ['web', 'prices.example.com'],
      ['file', 'Quote A.pdf'],
    ])
  })
  test('reads through the gateway call', () => {
    const list = sourcesOf([step('hq_call', { tool: 'vault_read', args: { path: 'Notebooks/Plan.md' } })])
    expect(list).toHaveLength(1)
    expect(list[0].ref).toBe('Notebooks/Plan.md')
  })
  test('a step still running marks its source as being read', () => {
    expect(sourcesOf([step('vault_read', { path: 'Notebooks/A.md' }, 'running')])[0].reading).toBe(true)
  })
  test('skips a call that names nothing, and searches', () => {
    expect(sourcesOf([step('vault_read'), step('web_search', { query: 'x' }), step('web_fetch', { url: 'not a url' })])).toEqual([])
  })
})

describe('what a reply made', () => {
  test('a note written under the Vault is a result the pane can open', () => {
    expect(madeNotes([step('vault_write_note', { path: 'Notebooks/Daily/Samsung steps.md' })])).toEqual([{ title: 'Samsung steps', path: 'Notebooks/Daily/Samsung steps.md' }])
  })
  test('ignores edits, paths outside the notes and repeats', () => {
    const made = madeNotes([
      step('vault_append_note', { path: 'Notebooks/A.md' }),
      step('vault_write_note', { path: '_system/x.md' }),
      step('vault_write_note', { path: 'Notebooks/B.md' }),
      step('vault_write_note', { path: 'Notebooks/B.md' }),
      step('vault_write_note', { path: 'Notebooks/Report.html' }),
    ])
    expect(made.map((m) => m.path)).toEqual(['Notebooks/B.md', 'Notebooks/Report.html'])
  })

  test('chatMadeItems collects notes and files across all assistant replies', () => {
    const items = chatMadeItems([
      {
        role: 'assistant',
        toolSteps: [step('vault_write_note', { path: 'Notebooks/Report.html' })],
        files: [{ path: 'Exports/2026-11/Budget.xlsx', name: 'Budget.xlsx', mime: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', kind: 'sheet', size: 2048 }],
      },
    ])
    expect(items.notes).toEqual([{ title: 'Report.html', path: 'Notebooks/Report.html' }])
    expect(items.files.map((f) => f.name)).toEqual(['Budget.xlsx'])
  })
})
