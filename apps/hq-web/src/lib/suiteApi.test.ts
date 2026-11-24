import { afterEach, expect, test } from 'bun:test'
import {
  cardAdd,
  cardDelete,
  cardMove,
  cardReplace,
  cardSetNotes,
  cardSetStyle,
  dashAddPlot,
  dashAddTile,
  fetchSuiteCards,
  sheetAdd,
  sheetAddColumn,
  sheetAddRows,
  sheetDeleteRows,
  sheetSetCells,
  sheetSetColumnType,
  suiteEdit,
  suiteSourceOf,
} from './suiteApi'

const note = (front: string) => `---\n${front}\n---\n\nBody`

test('a note made from an office file names its source', () => {
  expect(suiteSourceOf(note('title: A\nsource: Notebooks/Inbox/Imports/A.docx\nsource_sha256: abc'))).toBe(
    'Notebooks/Inbox/Imports/A.docx',
  )
  expect(suiteSourceOf(note('source: "Notebooks/a b.docx"\nsource_sha256: abc'))).toBe('Notebooks/a b.docx')
})

test('any other note, and a note whose original is gone, has no source', () => {
  expect(suiteSourceOf('Just text')).toBeNull()
  expect(suiteSourceOf(note('title: A'))).toBeNull()
  expect(suiteSourceOf(note('source: a.docx'))).toBeNull()
  expect(suiteSourceOf(note('source: a.docx\nsource_sha256: abc\nsuite_original_state: gone'))).toBeNull()
  expect(suiteSourceOf(note('source: a.docx\nsource_sha256: abc\nsuite_original_state: trash'))).toBe('a.docx')
  expect(suiteSourceOf('---\nsource: a.docx\nsource_sha256: abc\nno end')).toBeNull()
})

// ---- edits

const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

interface Sent {
  url: string
  method: string
  body: Record<string, unknown> | null
}

/** Answers every call with `reply` (status 200, or `status`) and records what was sent. */
function mockFetch(reply: unknown, status = 200): Sent[] {
  const sent: Sent[] = []
  globalThis.fetch = (async (url: string, init: RequestInit = {}) => {
    sent.push({ url: String(url), method: init.method ?? 'GET', body: init.body ? JSON.parse(String(init.body)) : null })
    return new Response(JSON.stringify(reply), { status, headers: { 'content-type': 'application/json' } })
  }) as unknown as typeof fetch
  return sent
}

const ok = { changed: true, summary: 'Done', revision: 'r1' }

test('an edit posts the note as path and the action with its arguments in snake_case', async () => {
  const sent = mockFetch(ok)
  const out = await suiteEdit('Notebooks/a.md', 'add_sheet', { name: 'P', afterIndex: 2, hash: 'h', types: undefined })
  expect(out.changed).toBe(true)
  expect(sent).toHaveLength(1)
  expect(sent[0].url).toBe('/api/suite/edit')
  expect(sent[0].method).toBe('POST')
  expect(sent[0].body).toEqual({ path: 'Notebooks/a.md', action: 'add_sheet', name: 'P', after_index: 2, hash: 'h' })
})

test('a dry run asks for the preview and sends nothing else new', async () => {
  const sent = mockFetch({ preview: true, summary: 'Would delete', revision: 'r1', touched: 4, destructive: true })
  const out = await sheetDeleteRows('Notebooks/a.md', { hash: 'h', row: 2 }, true)
  expect(out.preview).toBe(true)
  expect(out.destructive).toBe(true)
  expect(sent[0].body).toEqual({ path: 'Notebooks/a.md', action: 'delete_rows', hash: 'h', row: 2, dry_run: true })
})

test('every sheet edit names its action and carries its arguments', async () => {
  const sent = mockFetch(ok)
  await sheetSetCells('n.md', { row: 1, col: 0, values: [['a', 2]], expected: [['', '']] })
  await sheetAddRows('n.md', { hash: 'h', rows: [['x', 1]], at: 2 })
  await sheetDeleteRows('n.md', { hash: 'h', row: 1, count: 2 })
  await sheetAddColumn('n.md', { hash: 'h', name: 'paid', type: 'bool', default: 'no' })
  await sheetSetColumnType('n.md', { hash: 'h', col: 1, type: 'ugx' })
  await sheetAdd('n.md', { name: 'Prices', columns: ['item', 'price'], types: ['text', 'ugx'], rows: [['Sugar', 4200]] })
  expect(sent.map((s) => s.body?.action)).toEqual(['set_cells', 'add_rows', 'delete_rows', 'add_column', 'set_column_type', 'add_sheet'])
  expect(sent[0].body).toMatchObject({ row: 1, col: 0, values: [['a', 2]], expected: [['', '']] })
  expect(sent[1].body).toMatchObject({ at: 2, rows: [['x', 1]] })
  expect(sent[3].body).toMatchObject({ name: 'paid', type: 'bool', default: 'no' })
  expect(sent[5].body).toMatchObject({ name: 'Prices', columns: ['item', 'price'], types: ['text', 'ugx'] })
})

test('every card edit names its action, and a style key left null is sent so it clears', async () => {
  const sent = mockFetch(ok)
  await cardAdd('d.md', { title: 'Risks', markdown: '- a', afterCard: 0, hash: 'h' })
  await cardReplace('d.md', { card: 1, hash: 'h', markdown: '- b' })
  await cardMove('d.md', { card: 2, hash: 'h', to: 0 })
  await cardDelete('d.md', { card: 1, hash: 'h' })
  await cardSetNotes('d.md', { card: 0, hash: 'h', text: 'Say hi' })
  await cardSetStyle('d.md', { card: 0, hash: 'h', transition: 'fade slow', background: '#1F3A5F', layout: null })
  expect(sent.map((s) => s.body?.action)).toEqual(['add_card', 'replace_card', 'move_card', 'delete_card', 'set_notes', 'set_card_style'])
  expect(sent[0].body).toMatchObject({ after_card: 0, title: 'Risks' })
  expect(sent[5].body).toMatchObject({ transition: 'fade slow', background: '#1F3A5F', layout: null })
  expect(sent[5].body).not.toHaveProperty('color')
})

test('a tile and a plot are built from a range and its columns', async () => {
  const sent = mockFetch({ ...ok, tile: { label: 'Units', text: '75', cells: 3 } })
  const out = await dashAddTile('s.md', { from: 'C2:C4', fn: 'sum', label: 'Units', format: 'int' })
  expect((out.tile as { text: string }).text).toBe('75')
  await dashAddPlot('s.md', { type: 'bar', from: 'A1:D4', x: 'item', y: 'qty', group: 'region', title: 'Stock', afterIndex: 1, hash: 'h' })
  expect(sent[0].body).toMatchObject({ action: 'add_tile', from: 'C2:C4', fn: 'sum' })
  expect(sent[1].body).toMatchObject({ action: 'add_plot', type: 'bar', x: 'item', y: 'qty', group: 'region', after_index: 1 })
})

test('the cards of a deck are read with their path encoded', async () => {
  const sent = mockFetch({ revision: 'r', title: 'T', cards: [{ index: 0, title: 'A', hash: 'h', blocks: 2, layout: 'content', transition: null, background: null, color: null, notes: [] }] })
  const out = await fetchSuiteCards('Notebooks/my deck.md')
  expect(sent[0].url).toBe('/api/suite/cards?path=Notebooks%2Fmy%20deck.md')
  expect(sent[0].method).toBe('GET')
  expect(out.cards[0].title).toBe('A')
})

test("a refusal rejects with hq's sentence", async () => {
  mockFetch({ error: 'that sheet changed since you read it; read the note again and retry' }, 422)
  await expect(sheetAddRows('n.md', { hash: 'old', rows: [['x']] })).rejects.toThrow('changed since you read it')
})
