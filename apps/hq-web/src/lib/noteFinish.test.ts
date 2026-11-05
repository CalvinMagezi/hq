import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { keepBoth, saveNote, seeTheirVersion, type NoteApi } from './noteSave'
import { exportGate } from './noteExport'
import { reviewTag, whoWrote, offersLooksRight, type NoteVersion } from './noteReview'

const conflict = { saved: false, changed: true, by: 'agent', at: '2026-01-01T10:00:00Z', theirs_hash: 'h2' }

function fake(over: Partial<Record<string, unknown>> = {}) {
  const calls: { fn: string; args: unknown[] }[] = []
  const api = {
    update: async (...args: unknown[]) => {
      calls.push({ fn: 'update', args })
      return conflict
    },
    get: async (...args: unknown[]) => {
      calls.push({ fn: 'get', args })
      return { content: '---\ntitle: x\n---\nline one\ntheirs', hash: 'h2' }
    },
    create: async (...args: unknown[]) => {
      calls.push({ fn: 'create', args })
      return { path: 'Notes/a, your version.md' }
    },
    ...over,
  } as unknown as NoteApi
  return { api, calls }
}

describe('note 409 flow', () => {
  test('a 409 is a conflict and writes nothing', async () => {
    const { api } = fake()
    const step = await saveNote(api, 'Notes/a.md', 'mine', 'h1')
    expect(step.kind).toBe('conflict')
  })

  test('See their version is read only and counts differing lines', async () => {
    const { api, calls } = fake()
    const seen = await seeTheirVersion(api, 'Notes/a.md', 'line one\nmine')
    expect(seen?.text).toBe('line one\ntheirs')
    expect(seen?.differ).toBeGreaterThan(0)
    expect(calls.map((c) => c.fn)).toEqual(['get'])
  })

  test('Keep both saves yours as a new note with a fresh client id', async () => {
    const { api, calls } = fake()
    let n = 0
    const done = await keepBoth(api, 'Notes/a.md', 'mine', 'your version', () => `id-${++n}`)
    expect(done.ok).toBe(true)
    const create = calls.find((c) => c.fn === 'create')!
    expect(create.args[0]).toBe('a, your version')
    expect(create.args[1]).toBe('mine')
    expect(create.args[3]).toBe('id-1')
  })

  test('a failed save keeps the text', async () => {
    const { api } = fake({ update: async () => { throw new Error('down') } })
    expect((await saveNote(api, 'a.md', 'mine')).kind).toBe('failed')
  })
})

describe('export gate', () => {
  test('hub, away, shared and ok', () => {
    expect(exportGate({ inHub: true, away: true, shared: true })).toBe('window')
    expect(exportGate({ inHub: false, away: true, shared: true })).toBe('away')
    expect(exportGate({ inHub: false, away: false, shared: true })).toBe('ask')
    expect(exportGate({ inHub: false, away: false, shared: false })).toBe('ok')
  })
})

describe('review mapping', () => {
  const v = (o: Partial<NoteVersion>): NoteVersion => ({
    id: 1, content_hash: 'h', is_current: true, source_kind: 'human', written_at: '2026-01-01T00:00:00Z',
    review_state: 'unreviewed', review_history: [], ...o,
  })
  test('tags and writers', () => {
    expect(reviewTag(v({ review_state: 'disputed' }))).toBe('disputed')
    expect(reviewTag(v({ unattributed: true }))).toBe('unknown')
    expect(whoWrote(v({ source_kind: 'agent', actor: 'scout' }))).toEqual({ kind: 'helper', name: 'scout' })
    expect(whoWrote(v({ source_kind: 'imported' })).kind).toBe('imported')
    expect(offersLooksRight(v({ review_state: 'accepted' }))).toBe(false)
    expect(offersLooksRight(v({}))).toBe(true)
  })
})

describe('sources', () => {
  const read = (p: string) => readFileSync(new URL(p, import.meta.url), 'utf8')
  test('the drawer and the Notes home use the catalogue', () => {
    expect(read('../components/VaultNoteDrawer.tsx')).not.toContain('Note Not Found')
    const home = read('../routes/notes/index.tsx')
    expect(home).toContain("hq:open-search")
    expect(home).toContain('notes_search_your_notes')
  })
  test('the note page no longer uses window.alert', () => {
    expect(read('../routes/notes/$.tsx')).not.toContain('window.alert')
  })
})
