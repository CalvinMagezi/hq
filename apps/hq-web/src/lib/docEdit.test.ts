import { describe, expect, test } from 'bun:test'
import { appendBlock, countMatches, imageMarkdown, isImagePath, isOneBlock, outline, relativePath, replaceAll, tableMarkdown } from './docEdit'

const DOC = '---\nsuite_kind: doc\ntitle: Cat story\n---\n# Cats\n\nThe cat sat. Cat naps.\n\n```\n# not a heading\n```\n\n## Dogs\n\nNo cat here, a dog.\n\n### Deep ###\n'

describe('outline', () => {
  test('headings in order with levels, code and frontmatter left out', () => {
    expect(outline(DOC)).toEqual([
      { level: 1, text: 'Cats', n: 0 },
      { level: 2, text: 'Dogs', n: 1 },
      { level: 3, text: 'Deep', n: 2 },
    ])
  })
  test('a note with none gives an empty outline', () => {
    expect(outline('just words')).toEqual([])
  })
})

describe('find and replace', () => {
  test('counts matches in the body only, case blind unless asked', () => {
    expect(countMatches(DOC, 'cat')).toBe(4)
    expect(countMatches(DOC, 'cat', { caseSensitive: true })).toBe(2)
    expect(countMatches(DOC, '')).toBe(0)
  })
  test('characters that mean something in a pattern are plain text', () => {
    expect(countMatches('a.b a+b (x)', '.')).toBe(1)
    expect(countMatches('a.b a+b (x)', '(x)')).toBe(1)
  })
  test('replaces every match, leaves the frontmatter alone, and a replacement with $ is literal', () => {
    const out = replaceAll(DOC, 'cat', 'fox $&')!
    expect(out.startsWith('---\nsuite_kind: doc\ntitle: Cat story\n---\n')).toBe(true)
    expect(out).toContain('The fox $& sat. fox $& naps.')
    expect(countMatches(out, 'cat')).toBe(0)
  })
  test('nothing to replace gives null', () => {
    expect(replaceAll(DOC, 'zebra', 'x')).toBeNull()
    expect(replaceAll(DOC, '', 'x')).toBeNull()
  })
})

describe('table and image', () => {
  test('a table of the asked size, headers numbered, bounded', () => {
    expect(tableMarkdown(2, 2)).toBe('| Column 1 | Column 2 |\n| --- | --- |\n|  |  |\n|  |  |')
    expect(tableMarkdown(2, 1, ['Item', 'Qty|x']).split('\n')[0]).toBe('| Item | Qty\\|x |')
    expect(tableMarkdown(99, 999).split('\n')).toHaveLength(2 + 50)
    expect(tableMarkdown(0, 0).split('\n')).toHaveLength(3)
  })
  test('a block is added at the end, one blank line away, in the note line endings', () => {
    expect(appendBlock('a\n', 'b')).toBe('a\n\nb\n')
    expect(appendBlock('a\r\n\r\n', 'b\nc')).toBe('a\r\n\r\nb\r\nc\r\n')
  })
  test('pictures are found by extension and written relative to the note', () => {
    expect(isImagePath('a/b.PNG')).toBe(true)
    expect(isImagePath('a/b.md')).toBe(false)
    expect(relativePath('Notebooks/Docs/Plan.md', 'Notebooks/Docs/img/a.png')).toBe('img/a.png')
    expect(relativePath('Notebooks/Docs/Plan.md', 'Notebooks/Other/a.png')).toBe('../Other/a.png')
    expect(relativePath('Plan.md', '_media/a.png')).toBe('_media/a.png')
    expect(imageMarkdown('A [cat]', 'my pics/a b.png')).toBe('![A cat](my%20pics/a%20b.png)')
  })
})

describe('one block', () => {
  test('a paragraph or a list is one block; blank-line separated text is two', () => {
    expect(isOneBlock('A line\nand more')).toBe(true)
    expect(isOneBlock('- a\n- b')).toBe(true)
    expect(isOneBlock('One\n\nTwo')).toBe(false)
  })
  test('a fenced block may hold blank lines, an unclosed fence is refused', () => {
    expect(isOneBlock('```\na\n\nb\n```')).toBe(true)
    expect(isOneBlock('```\na')).toBe(false)
  })
})
