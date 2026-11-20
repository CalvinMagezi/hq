import { describe, expect, test } from 'bun:test'
import { chatMadeItems, madeNotes, relevantSources, sourcesOf, splitSources } from './replySources'

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

  test('relevantSources filters out untouched/uncited notes while keeping cited or written notes', () => {
    const steps = [
      step('vault_read', { path: 'Notebooks/Projects/Unrelated Scope.md' }),
      step('vault_read', { path: 'Notebooks/Shop/Stock rules.md' }),
      step('vault_read', { path: 'Notebooks/Daily/Dashboard.md' }),
      step('vault_write_note', { path: 'Notebooks/Daily/Dashboard.md' }),
      step('web_fetch', { url: 'https://www.prices.example.com/quote?id=9' }),
    ]
    const answer = 'I built your dashboard using `Notebooks/Shop/Stock rules.md`.'
    const filtered = relevantSources(steps, answer)
    expect(filtered.map((s) => s.ref)).toEqual([
      'Notebooks/Shop/Stock rules.md',
      'Notebooks/Daily/Dashboard.md',
      'https://www.prices.example.com/quote?id=9',
    ])
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

describe('source chips', () => {
  const fetchOf = (url: string) => step('web_fetch', { url })

  test('one page linked in different ways is listed once', () => {
    const list = sourcesOf([
      fetchOf('https://www.farmgain.example/prices/matooke?utm_source=x#top'),
      fetchOf('https://farmgain.example/prices/matooke/'),
      fetchOf('http://farmgain.example/prices/matooke?fbclid=9'),
    ])
    expect(list).toHaveLength(1)
    expect(list[0].ref).toBe('https://www.farmgain.example/prices/matooke?utm_source=x#top')
  })

  test('keeps real query parameters apart', () => {
    expect(sourcesOf([fetchOf('https://a.example/p?id=1'), fetchOf('https://a.example/p?id=2')])).toHaveLength(2)
  })

  test('two pages of one site are labelled by their path tail, a lone page by its domain', () => {
    const list = sourcesOf([fetchOf('https://farmgain.example/prices/matooke-2024'), fetchOf('https://farmgain.example/news/market-day'), fetchOf('https://other.example/x')])
    expect(list.map((s) => s.title)).toEqual(['farmgain.example: matooke 2024', 'farmgain.example: market day', 'other.example'])
  })

  test('when the reply cites some sources, only those show', () => {
    const steps = [fetchOf('https://farmgain.example/a'), fetchOf('https://market.example/b'), fetchOf('https://noise.example/c')]
    const answer = 'Prices are up ([Source](https://farmgain.example/a)). The market.example team agrees.'
    expect(relevantSources(steps, answer).map((s) => s.ref)).toEqual(['https://farmgain.example/a', 'https://market.example/b'])
  })

  test('a linked page of a site does not cite its other pages', () => {
    const steps = [fetchOf('https://farmgain.example/a'), fetchOf('https://farmgain.example/b')]
    expect(relevantSources(steps, 'See [Source](https://farmgain.example/a/)').map((s) => s.ref)).toEqual(['https://farmgain.example/a'])
  })

  test('when nothing is cited, every source stays', () => {
    const steps = [fetchOf('https://a.example/1'), fetchOf('https://b.example/2')]
    expect(relevantSources(steps, 'Prices are up.')).toHaveLength(2)
  })

  test('the cap shows five and counts the rest; expanding shows all', () => {
    const list = Array.from({ length: 12 }, (_, i) => ({ kind: 'web' as const, title: `s${i}`, ref: `https://s${i}.example`, reading: false }))
    expect(splitSources(list, false)).toMatchObject({ hidden: 7 })
    expect(splitSources(list, false).shown).toHaveLength(5)
    expect(splitSources(list, true)).toMatchObject({ hidden: 0 })
    expect(splitSources(list, true).shown).toHaveLength(12)
    expect(splitSources(list.slice(0, 5), false).hidden).toBe(0)
  })
})
