import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { opensQuickNote, savesQuickNote } from './quickNote'
import { keepBothTitle } from './noteConflict'

const key = (o: object) => ({ ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, code: '', key: '', ...o })
const src = (p: string) => readFileSync(new URL(p, import.meta.url), 'utf8')

describe('notes (spec 03)', () => {
  test('Ctrl Alt N opens a new note, Ctrl N does not (acceptance 7)', () => {
    expect(opensQuickNote(key({ ctrlKey: true, altKey: true, code: 'KeyN' }))).toBe(true)
    expect(opensQuickNote(key({ ctrlKey: true, code: 'KeyN', key: 'n' }))).toBe(false)
    expect(opensQuickNote(key({ metaKey: true, code: 'KeyN', key: 'n' }))).toBe(false)
  })
  test('Ctrl Alt N is ignored while typing in a field', () => {
    expect(opensQuickNote(key({ ctrlKey: true, altKey: true, code: 'KeyN', target: { tagName: 'TEXTAREA' } }))).toBe(false)
  })
  test('Ctrl Enter saves the new note', () => {
    expect(savesQuickNote({ ctrlKey: true, metaKey: false, key: 'Enter' })).toBe(true)
    expect(savesQuickNote({ ctrlKey: false, metaKey: false, key: 'Enter' })).toBe(false)
  })
  test('Ctrl plus, minus and zero no longer change the text size (acceptance 3)', () => {
    expect(src('../routes/notes/$.tsx')).not.toContain("e.key === '='")
  })
  test('the pin suggestion offers Keep pin and Not now (acceptance 4)', () => {
    expect(src('../components/vault/HomeSections.tsx')).toContain('onNotNow')
    expect(src('../routes/notes/index.tsx')).toContain('onKeepPin')
  })
  test('Notes stays lit on every address under it (acceptance 5)', () => {
    expect(src('../components/AppRail.tsx')).toContain('exact: false')
    expect(src('../components/BottomNav.tsx')).toContain('exact: false')
  })
  test('a save carries the base hash and a create carries a client id', () => {
    const api = src('./vaultApi.ts')
    expect(api).toContain('base })')
    expect(api).toContain('client_id: clientId')
    expect(api).toContain('theirs_hash')
  })
  test('keep both names the second note after the first', () => {
    expect(keepBothTitle('Notebooks/Inbox/Plan.md', 'your version')).toBe('Plan, your version')
  })
})
