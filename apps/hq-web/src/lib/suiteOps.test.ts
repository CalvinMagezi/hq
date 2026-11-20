import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { suiteOps } from './suiteOps'

const calls: { url: string; body?: unknown }[] = []
const blocks = [
  { index: 0, hash: 'aaaaaaaaaaaa', kind: 'heading 1', src: '# T' },
  { index: 1, hash: 'bbbbbbbbbbbb', kind: 'sheet', src: '```sheet\n| a | b |\n```' },
  { index: 2, hash: 'cccccccccccc', kind: 'sheet', src: '```sheet\n| c |\n```' },
]
let fail: { status: number; error: string } | null = null
const realFetch = globalThis.fetch

beforeEach(() => {
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    calls.push({ url, body: init?.body ? JSON.parse(String(init.body)) : undefined })
    if (url.startsWith('/api/suite/blocks')) return new Response(JSON.stringify({ revision: 'r', blocks }))
    if (fail) return new Response(JSON.stringify({ error: fail.error }), { status: fail.status })
    return new Response(JSON.stringify({ applied: true, summary: 'ok', revision: 'r2' }))
  }) as typeof fetch
})

afterEach(() => {
  globalThis.fetch = realFetch
  calls.length = 0
  fail = null
})

describe('sheet structure edits go to the one edit route', () => {
  test('a row is inserted before its table row, quoting the sheet block hash read just before', async () => {
    expect(await suiteOps.rows({ path: 'N/a.md', index: 1 }, 'insert', 3)).toEqual({ ok: true })
    expect(calls[0].url).toStartWith('/api/suite/blocks')
    expect(calls[1].url).toBe('/api/suite/edit')
    // The second sheet block is number 1, whatever other blocks come before it.
    expect(calls[1].body).toMatchObject({ path: 'N/a.md', action: 'add_rows', sheet: 1, hash: 'cccccccccccc', at: 4, rows: [['']] })
  })

  test('a delete names the table row (1 is the first under the header) and a column goes on the right', async () => {
    await suiteOps.rows({ path: 'N/a.md', index: 0 }, 'delete', 0)
    expect(calls[1].body).toMatchObject({ action: 'delete_rows', sheet: 0, hash: 'bbbbbbbbbbbb', row: 1, count: 1 })
    await suiteOps.addColumn({ path: 'N/a.md', index: 0 }, 'Notes')
    expect(calls[3].body).toMatchObject({ action: 'add_column', name: 'Notes' })
    await suiteOps.columnType({ path: 'N/a.md', index: 0 }, 2, 'ugx')
    expect(calls[5].body).toMatchObject({ action: 'set_column_type', col: 2, type: 'ugx' })
  })

  test("HQ's sentence for a refused change comes back; a missing sheet is refused before any edit is sent", async () => {
    fail = { status: 422, error: 'that block changed since it was read' }
    const r = await suiteOps.rows({ path: 'N/a.md', index: 0 }, 'delete', 0)
    expect(r).toEqual({ ok: false, reason: 'stale', message: expect.stringContaining('changed') })
    calls.length = 0
    fail = null
    const none = await suiteOps.rows({ path: 'N/a.md', index: 5 }, 'insert', 0)
    expect(none.ok).toBe(false)
    expect(calls.some((c) => c.url === '/api/suite/edit')).toBe(false)
  })
})
