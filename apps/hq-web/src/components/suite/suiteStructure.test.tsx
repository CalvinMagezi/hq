import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { SuiteViewer } from './SuiteViewer'
import { DENSITY_KEY, readDensity, saveDensity } from './useDensity'
import { glance } from './SlidesView'

const SHEET = '```sheet name=Stock types="text,int,ugx,ugx" id=stock\n| item | qty | price | total |\n|---|---|---|---|\n| Sugar | 40 | 4200 | =B2*C2 |\n| Bad | 1 | 1 | =NOPE(1) |\n```\n'
const SALES = '```sheet name=Sales types="text,text,int"\n| month | region | amount |\n|---|---|---|\n| Jan | North | 10 |\n| Jan | South | 20 |\n| Feb | North | 30 |\n| Feb | South | 5 |\n```\n'
const draw = (text: string) => renderToStaticMarkup(createElement(SuiteViewer, { text }))
const editing = (text: string) => renderToStaticMarkup(createElement(SuiteViewer, { text, path: 'Notebooks/S.md', hash: 'h' }))

describe('the sheet as a grid', () => {
  test('is a grid of rows and cells with a labelled formula field, a toolbar and the view options', () => {
    const html = editing(`---\nsuite_kind: sheet\n---\n${SHEET}`)
    expect(html).toContain('role="grid"')
    expect(html).toContain('role="row"')
    expect(html).toContain('role="gridcell"')
    expect(html).toContain('role="columnheader"')
    expect(html).toContain('role="rowheader"')
    expect(html).toContain('aria-selected="false"')
    expect(html).toContain('aria-live="polite"')
    expect(html).toContain('aria-label="Formula"')
    expect(html).toContain('>View<')
  })
  test('a read-only sheet has a disabled formula field and no row button', () => {
    const html = draw(`---\nsuite_kind: sheet\n---\n${SHEET}`)
    expect(html).toContain('disabled=""')
    expect(html).not.toContain('Add a row')
  })
})

describe('slides as a 16:9 stage and a strip of small cards', () => {
  const DECK = '---\nsuite_kind: slides\n---\n# D\n\n## One\n<!-- background: #1F3A5F -->\n<!-- color: #FFFFFF -->\n<!-- notes: Say hi -->\n\nHello there\n\n## Two\n\nWorld\n'
  const withPath = (text: string) => renderToStaticMarkup(createElement(SuiteViewer, { text, path: 'Notebooks/D.md', hash: 'h' }))
  test('each card is a small picture of the card with its own colours, and the stage takes them too', () => {
    const html = draw(DECK)
    expect(html).toContain('su-mini')
    expect(html).toContain('su-stage')
    expect(html).toContain('background:#1F3A5F')
    expect(html).toContain('Hello there')
    // The comments are not drawn as card text.
    expect(html).not.toContain('<!--')
  })
  test('with a place to save, the card tools, layout, transition, colour and notes fields appear', () => {
    const html = withPath(DECK)
    for (const word of ['Add a card', 'Copy this card', 'Delete this card', 'Move later', 'Two columns', 'Transition', 'Background colour', 'Text colour', 'Speaker notes']) expect(html).toContain(word)
    expect(html).toContain('Say hi')
    expect(html).not.toContain('hard to read')
  })
  test('white text on a white background warns', () => {
    expect(withPath(DECK.replace('#1F3A5F', '#FFFFFF'))).toContain('hard to read')
  })
  test('a reader sees the notes in a closed disclosure', () => {
    expect(draw(DECK)).toContain('<details')
  })
  test('the small picture shows the first words as plain text', () => {
    expect(glance('## Head\n\n- **Bold** item\n- [link](x) two\n- three\n- four', 2)).toBe('Head · Bold item')
  })
})

describe('the dashboard', () => {
  const DASH = `---\nsuite_kind: dashboard\n---\n${SALES}\n\`\`\`tile from=C2:C5 fn=sum label="Total" trend=yes\n\`\`\`\n\n\`\`\`plot type=bar from=A1:C5 x=month y=amount group=region title="By month"\n\`\`\`\n`
  test('filters from a column, a tile with its trend, and a chart with axes, two series and a legend', () => {
    const html = draw(DASH)
    expect(html).toContain('Filters')
    expect(html).toContain('<option value="North">North</option>')
    expect(html).toContain('su-delta')
    expect(html).toContain('su-spark')
    expect(html).toContain('su-axis')
    expect(html).toContain('North')
    expect(html).toContain('South')
    expect(html).toContain('Tap the chart')
    expect(html).toContain('only what you see here')
  })
  test('a tile shows no trend unless it asks for one', () => {
    const html = draw(DASH.replace(' trend=yes', ''))
    expect(html).not.toContain('su-delta')
    expect(html).not.toContain('su-spark')
  })
  test('no data colour is the border colour', () => {
    const html = draw(DASH)
    expect(html).not.toMatch(/(fill|stroke)="var\(--line\)"/)
    expect(html).not.toContain('var(--hairline)"')
  })
  test('a pie still draws its slices in the shared palette', () => {
    const html = draw(DASH.replace('type=bar', 'type=pie').replace(' group=region', ''))
    expect(html).toContain('su-slice')
    expect(html).not.toMatch(/fill="var\(--line\)"/)
  })
})

describe('grid density is kept on this device only', () => {
  test('a blocked store gives comfortable and never throws', () => {
    expect(readDensity()).toBe('comfortable')
    expect(() => saveDensity('compact')).not.toThrow()
    expect(DENSITY_KEY).toBe('hq.suite.density')
  })
  test('a saved choice is read back', () => {
    const store = new Map<string, string>()
    ;(globalThis as { localStorage?: unknown }).localStorage = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) }
    try {
      saveDensity('compact')
      expect(readDensity()).toBe('compact')
    } finally {
      delete (globalThis as { localStorage?: unknown }).localStorage
    }
  })
})
