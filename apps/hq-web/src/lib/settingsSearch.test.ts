import { describe, expect, test } from 'bun:test'
import { t } from '~/strings'
import { searchSettings, settingsEntries } from './settingsSearch'
import { readPaneMode, writePaneMode } from './paneMode'

const HERE = { lite: false, hub: false }
const find = (q: string, where = HERE) => searchSettings(where, q, (k) => t(k)).map((h) => h.row.id)

describe('searching Settings', () => {
  test('finds a row by its name, its group or the line under it', () => {
    expect(find('spending')).toContain('spending')
    expect(find('enter')).toEqual(expect.arrayContaining(['chat']))
    expect(find('openrouter')).toEqual(['keys'])
    expect(find('chats and devices')).toEqual(expect.arrayContaining(['chat', 'messaging', 'devices', 'github']))
  })
  test('every word must match, and nothing typed finds nothing', () => {
    expect(find('enter pane')).toEqual(['chat'])
    expect(find('enter zebra')).toEqual([])
    expect(find('   ')).toEqual([])
  })
  test('only offers rows this HQ has', () => {
    expect(find('enter', { lite: true, hub: false })).toEqual([])
    expect(settingsEntries(HERE).every((h) => h.hint !== undefined)).toBe(true)
  })
})

describe('the work pane choice', () => {
  const store = () => {
    const data: Record<string, string> = {}
    return { getItem: (k: string) => data[k] ?? null, setItem: (k: string, v: string) => void (data[k] = v) }
  }
  test('is automatic until chosen, and a damaged value reads as automatic', () => {
    const s = store()
    expect(readPaneMode(s)).toBe('auto')
    writePaneMode('never', s)
    expect(readPaneMode(s)).toBe('never')
    s.setItem('hq-pane-mode', 'sideways')
    expect(readPaneMode(s)).toBe('auto')
    expect(readPaneMode({ getItem: () => { throw new Error('blocked') } })).toBe('auto')
  })
})
