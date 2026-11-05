import { expect, test } from 'bun:test'
import { foundParts, GROUP_SIZE, SEARCH_DEBOUNCE_MS, RECENT_MAX, SCOPES, LITE_SCOPES, visibleRows, withRecent, readRecent, writeRecent, searchFailureLine, topFolders } from './search'
import { searchPath } from './vaultApi'
import { t } from '~/strings'

function memory(): Storage {
  const m = new Map<string, string>()
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v) } as Storage
}

test('found words are split out to be bold, case blind', () => {
  expect(foundParts('Budget plan 2026', 'plan')).toEqual([
    { text: 'Budget ', found: false },
    { text: 'plan', found: true },
    { text: ' 2026', found: false },
  ])
})

test('regex characters in a query are plain text', () => {
  expect(foundParts('a (b) c', '(b)').some((p) => p.found && p.text === '(b)')).toBe(true)
})

test('search waits 300 ms and has five scopes, three in Lite', () => {
  expect(SEARCH_DEBOUNCE_MS).toBe(300)
  expect(SCOPES).toEqual(['all', 'notes', 'tasks', 'chats', 'helpers'])
  expect(LITE_SCOPES).toEqual(['all', 'notes', 'tasks'])
})

test('a group shows five rows until Show all', () => {
  const rows = [1, 2, 3, 4, 5, 6, 7]
  expect(visibleRows(rows, false)).toHaveLength(GROUP_SIZE)
  expect(visibleRows(rows, true)).toHaveLength(7)
})

test('the folder travels to the server only when set', () => {
  expect(searchPath('a b')).not.toContain('folder')
  expect(searchPath('a', 'Notes/Work')).toContain('&folder=Notes%2FWork')
})

test('recent searches: newest first, no repeats, eight at most, kept per HQ', () => {
  let list: string[] = []
  for (let i = 0; i < 10; i++) list = withRecent(list, `q${i}`)
  expect(list).toHaveLength(RECENT_MAX)
  expect(withRecent(['a', 'b'], 'B')).toEqual(['B', 'a'])
  const store = memory()
  writeRecent('one', ['x'], store)
  expect(readRecent('one', store)).toEqual(['x'])
  expect(readRecent('two', store)).toEqual([])
})

test('a failed notes search says why, by connection state', () => {
  expect(searchFailureLine('offline', false)).toBe(t('fail_search_offline'))
  expect(searchFailureLine('unreachable', false)).toBe(t('fail_search_unreachable'))
})

test('folder facets count hits per top folder', () => {
  const hits = [{ notePath: 'A/x.md' }, { notePath: 'A/y.md' }, { notePath: 'B/z.md' }] as never
  expect(topFolders(hits)[0]).toEqual({ folder: 'A', count: 2 })
})
