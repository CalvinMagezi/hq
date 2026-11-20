import { beforeEach, describe, expect, test } from 'bun:test'
import { OP_ROUTES, createSuiteOps, opAvailable, probeSuiteOps, resetSuiteOpsProbe } from './suiteOps'

const target = { path: 'N/a.md', base: 'h1', index: 0 }
const reply = (status: number, body: unknown = {}) => new Response(JSON.stringify(body), { status })

beforeEach(() => resetSuiteOpsProbe())

describe('the planned structure routes', () => {
  test('a call posts its body to the planned route with the note hash', async () => {
    const seen: { url: string; init?: RequestInit }[] = []
    const ops = createSuiteOps(async (url, init) => (seen.push({ url, init }), reply(200)))
    expect(await ops.rows(target, 'insert', 3)).toEqual({ ok: true })
    expect(seen[0].url).toBe('/api/suite/sheet/rows')
    expect(seen[0].init?.method).toBe('POST')
    expect(JSON.parse(String(seen[0].init?.body))).toEqual({ path: 'N/a.md', base: 'h1', index: 0, action: 'insert', at: 3, count: 1 })
    await ops.columnType(target, 2, 'ugx')
    expect(seen[1].url).toBe('/api/suite/sheet/column-type')
    await ops.style(target, { layout: 'two-column' })
    expect(seen[2].url).toBe('/api/suite/slides/style')
  })

  test('a 404 means the route is not there yet: it is reported unavailable and remembered', async () => {
    const ops = createSuiteOps(async () => reply(404))
    expect(opAvailable('rows')).toBe(true)
    expect(await ops.rows(target, 'delete', 1)).toEqual({ ok: false, reason: 'unavailable', message: '' })
    expect(opAvailable('rows')).toBe(false)
    expect(opAvailable('columns')).toBe(true)
  })

  test('a refusal that names a stale note carries the server sentence; an outage is a plain failure', async () => {
    const stale = createSuiteOps(async () => reply(409, { error: 'the note changed' }))
    expect(await stale.card(target, 'add')).toEqual({ ok: false, reason: 'stale', message: 'the note changed' })
    const down = createSuiteOps(async () => Promise.reject(new TypeError('offline')))
    expect(await down.move(target, 2)).toEqual({ ok: false, reason: 'failed', message: '' })
    expect(opAvailable('move')).toBe(true)
  })

  test('the probe asks with OPTIONS and hides only what answered 404', async () => {
    const asked: string[] = []
    await probeSuiteOps(async (url, init) => {
      asked.push(`${init?.method} ${url}`)
      return reply(url === OP_ROUTES.notes ? 404 : 405)
    })
    expect(asked.every((a) => a.startsWith('OPTIONS '))).toBe(true)
    expect(asked).toHaveLength(Object.keys(OP_ROUTES).length)
    expect(opAvailable('notes')).toBe(false)
    expect(opAvailable('rows')).toBe(true)
  })

  test('a probe that cannot reach HQ leaves every route shown', async () => {
    await probeSuiteOps(async () => Promise.reject(new TypeError('offline')))
    expect(Object.keys(OP_ROUTES).every((n) => opAvailable(n as keyof typeof OP_ROUTES))).toBe(true)
  })
})
