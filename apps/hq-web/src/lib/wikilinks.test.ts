import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { Marked } from 'marked'
import { parseWikilink, wikilinkExtension, type WikilinkToken } from './wikilinks'

interface CorpusLink {
  target: string
  syntax?: 'markdown'
  heading?: string
  block?: string
  alias?: string
  embed?: boolean
  in_frontmatter?: boolean
}
interface Corpus {
  cases: { name: string; text: string; links: CorpusLink[] }[]
}

// The corpus the server's link reader is tested against too: one file, no drift.
const corpus: Corpus = JSON.parse(
  readFileSync(join(import.meta.dir, '../../../../crates/hq-vault/tests/fixtures/links_cases.json'), 'utf8'),
)

/** A note is drawn without its frontmatter, so the test reads it the same way. */
function withoutFrontmatter(text: string): string {
  return text.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, '')
}

function wikilinksIn(text: string): WikilinkToken[] {
  const marked = new Marked({ extensions: [wikilinkExtension] })
  const found: WikilinkToken[] = []
  const walk = (tokens: any[]) => {
    for (const tok of tokens) {
      if (tok.type === 'wikilink') found.push(tok)
      if (tok.tokens) walk(tok.tokens)
      if (tok.items) walk(tok.items)
    }
  }
  walk(marked.lexer(withoutFrontmatter(text)))
  return found
}

describe('wikilinks match the shared corpus', () => {
  for (const c of corpus.cases) {
    // Frontmatter is cut off before a note is drawn, and a link to this same note has no
    // target to open, so the page shows it as plain text.
    const want = c.links.filter((l) => l.syntax !== 'markdown' && !l.in_frontmatter && l.target !== '')
    test(c.name, () => {
      const got = wikilinksIn(c.text).map((t) => t.link)
      expect(got.length).toBe(want.length)
      got.forEach((link, i) => {
        expect(link.target).toBe(want[i].target)
        expect(link.heading).toBe(want[i].heading ?? null)
        expect(link.block).toBe(want[i].block ?? null)
        expect(link.alias).toBe(want[i].alias ?? null)
        expect(link.embed).toBe(want[i].embed ?? false)
      })
    })
  }
})

describe('parseWikilink', () => {
  test('splits target, heading, block and label', () => {
    expect(parseWikilink('Folder/Note#Goals|the goals')).toMatchObject({
      target: 'Folder/Note',
      heading: 'Goals',
      alias: 'the goals',
      block: null,
    })
    expect(parseWikilink('Note#^a1b2')).toMatchObject({ target: 'Note', block: 'a1b2', heading: null })
  })

  test('a table pipe and stray spaces do not leak into the target', () => {
    expect(parseWikilink(' Note \\| label ')).toMatchObject({ target: 'Note', alias: 'label' })
  })

  test('a link to the same note is not a link to open', () => {
    expect(parseWikilink('#Goals')).toBeNull()
    expect(parseWikilink('  ')).toBeNull()
  })
})
