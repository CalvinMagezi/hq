import { afterEach, expect, test } from 'bun:test'
import {
  NOTE_EXPORT_OPTIONS,
  defaultExtension,
  exportFileName,
  exportOptionsFor,
  extensionFromDisposition,
  noteHasTables,
} from './noteExport'
import { fetchNoteExport } from './vaultApi'
import { HqHttpError } from './hqAuth'

const TABLE = '| a | b |\n|---|---|\n| 1 | 2 |\n'

test('a GFM table is found, with or without outer pipes and alignment colons', () => {
  expect(noteHasTables(TABLE)).toBe(true)
  expect(noteHasTables('a | b\n--- | ---\n1 | 2\n')).toBe(true)
  expect(noteHasTables('| a | b |\n|:--|--:|\n| 1 | 2 |\n')).toBe(true)
  expect(noteHasTables('text before\n\n' + TABLE + '\ntext after')).toBe(true)
})

test('prose, rules and frontmatter are not tables', () => {
  expect(noteHasTables('')).toBe(false)
  expect(noteHasTables('just words\n\n---\n\nmore words')).toBe(false)
  expect(noteHasTables('---\ntitle: x\ndescription: |\n  a | b\n---\nbody')).toBe(false)
  expect(noteHasTables('a pipe | in prose\nnext line')).toBe(false)
})

test('a table inside a fenced code block does not count', () => {
  expect(noteHasTables('```md\n' + TABLE + '```\n')).toBe(false)
  expect(noteHasTables('~~~\n' + TABLE + '~~~\n')).toBe(false)
  // ...and the fence closes, so a real table after it still does.
  expect(noteHasTables('```\ncode\n```\n\n' + TABLE)).toBe(true)
})

test('table formats are offered only for a note that has a table', () => {
  const formats = (md: string) => exportOptionsFor(md).map((o) => o.format)
  expect(formats('prose')).toEqual(['pdf', 'docx', 'html', 'md', 'png', 'svg'])
  expect(formats(TABLE)).toEqual(['pdf', 'docx', 'html', 'md', 'png', 'svg', 'xlsx', 'csv', 'json'])
})

test('every offered format is one the server accepts, with a usable extension', () => {
  const serverFormats = new Set(['pdf', 'png', 'svg', 'docx', 'html', 'md', 'csv', 'json', 'xlsx'])
  for (const o of NOTE_EXPORT_OPTIONS) {
    expect(serverFormats.has(o.format)).toBe(true)
    expect(o.extension).toMatch(/^[a-z0-9]+$/)
  }
  expect(defaultExtension('xlsx')).toBe('xlsx')
})

test('download names drop the folder and note extension, and cannot carry path characters', () => {
  expect(exportFileName('Notebooks/Inbox/Plan.md', 'docx')).toBe('Plan.docx')
  expect(exportFileName('Plan.markdown', 'pdf')).toBe('Plan.pdf')
  expect(exportFileName('Notebooks/a:b*c?.md', 'csv')).toBe('a-b-c-.csv')
  expect(exportFileName('Notebooks/.md', 'pdf')).toBe('note.pdf')
  expect(exportFileName('', 'pdf')).toBe('note.pdf')
})

test('the saved extension follows the server, so several tables arrive as a zip', () => {
  expect(extensionFromDisposition('attachment; filename="Plan.csv"; filename*=UTF-8\'\'Plan.csv', 'csv')).toBe('csv')
  expect(extensionFromDisposition('attachment; filename="Plan.zip"; filename*=UTF-8\'\'Plan.zip', 'csv')).toBe('zip')
  expect(extensionFromDisposition("attachment; filename*=UTF-8''Caf%C3%A9.xlsx", 'csv')).toBe('xlsx')
  expect(extensionFromDisposition('attachment; filename=plan.json', 'csv')).toBe('json')
})

test('a missing or odd Content-Disposition falls back to the requested format', () => {
  expect(extensionFromDisposition(null, 'pdf')).toBe('pdf')
  expect(extensionFromDisposition('attachment', 'pdf')).toBe('pdf')
  expect(extensionFromDisposition('attachment; filename="noextension"', 'pdf')).toBe('pdf')
  expect(extensionFromDisposition('attachment; filename="x.<script>"', 'pdf')).toBe('pdf')
  expect(extensionFromDisposition("attachment; filename*=UTF-8''%E0%A4%A", 'pdf')).toBe('pdf')
})

const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

test('fetchNoteExport asks for the note and format and reads the extension from the response', async () => {
  let asked = ''
  globalThis.fetch = (async (url: string) => {
    asked = String(url)
    return new Response('zipbytes', {
      status: 200,
      headers: { 'content-disposition': 'attachment; filename="Plan.zip"', 'content-type': 'application/zip' },
    })
  }) as unknown as typeof fetch
  const out = await fetchNoteExport('Notebooks/My Plan.md', 'csv')
  expect(asked).toBe('/api/note/export?path=Notebooks%2FMy%20Plan.md&format=csv')
  expect(out.extension).toBe('zip')
  expect(await out.blob.text()).toBe('zipbytes')
})

test('fetchNoteExport turns a server error into an HqHttpError carrying its message and status', async () => {
  globalThis.fetch = (async () =>
    new Response(JSON.stringify({ error: 'this note has no tables to export' }), {
      status: 400,
      headers: { 'content-type': 'application/json' },
    })) as unknown as typeof fetch
  const err = await fetchNoteExport('Plan.md', 'xlsx').catch((e) => e)
  expect(err).toBeInstanceOf(HqHttpError)
  expect(err.status).toBe(400)
  expect(err.message).toBe('this note has no tables to export')
})

test('fetchNoteExport survives an error body that is not JSON', async () => {
  globalThis.fetch = (async () => new Response('Bad gateway', { status: 502, statusText: 'Bad Gateway' })) as unknown as typeof fetch
  const err = await fetchNoteExport('Plan.md', 'pdf').catch((e) => e)
  expect(err).toBeInstanceOf(HqHttpError)
  expect(err.status).toBe(502)
})
