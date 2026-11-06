import { describe, expect, test } from 'bun:test'
import { copyNoteLink, noteLinkUrl } from './noteLink'

describe('Copy link (notes spec 03 acceptance 6)', () => {
  test('puts /notes/n/{id} on the clipboard, never the folder path', async () => {
    const copied: string[] = []
    const out = await copyNoteLink('Notebooks/Projects/Plan.md', {
      mint: async (path) => ({ id: '0123456789abcdef', path, hash: 'h2' }),
      copy: async (text) => { copied.push(text); return true },
      origin: 'https://hq.example.ts.net/',
    })
    expect(copied).toEqual(['https://hq.example.ts.net/notes/n/0123456789abcdef'])
    expect(copied[0]).not.toContain('Plan')
    expect(out.link.hash).toBe('h2')
  })
  test('an id is always one path segment', () => {
    expect(noteLinkUrl('http://127.0.0.1:5678', 'a/b')).toBe('http://127.0.0.1:5678/notes/n/a%2Fb')
  })
})
