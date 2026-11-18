import { describe, expect, test } from 'bun:test'
import { sizeLabel, toReplyFiles } from './replyFiles'

describe('toReplyFiles', () => {
  test('keeps well-formed entries and drops the rest', () => {
    const files = toReplyFiles([
      { path: 'Exports/2026-11/a.xlsx', name: 'a.xlsx', mime: 'x', kind: 'sheet', size: 2048, note: 'Notebooks/Office/a.md', fidelity: 'Kept.' },
      { path: 5, name: 'bad' },
      'text',
      { path: 'b.bin', name: 'b.bin', kind: 'weird' },
    ])
    expect(files?.map((f) => f.path)).toEqual(['Exports/2026-11/a.xlsx', 'b.bin'])
    expect(files?.[0].note).toBe('Notebooks/Office/a.md')
    expect(files?.[1].kind).toBe('file')
  })
  test('nothing usable is undefined', () => {
    expect(toReplyFiles(undefined)).toBeUndefined()
    expect(toReplyFiles([])).toBeUndefined()
  })
  test('sizes read in plain units', () => {
    expect([sizeLabel(10), sizeLabel(2048), sizeLabel(3 * 1024 * 1024)]).toEqual(['10 B', '2 KB', '3.0 MB'])
  })
})
