import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { SuiteViewer } from './SuiteViewer'

const SHEET = '```sheet name=Stock types="text,int,ugx,ugx" id=stock\n| item | qty | price | total |\n|---|---|---|---|\n| Sugar | 40 | 4200 | =B2*C2 |\n| Bad | 1 | 1 | =NOPE(1) |\n```\n'
const draw = (text: string) => renderToStaticMarkup(createElement(SuiteViewer, { text }))

describe('the suite viewers', () => {
  test('a sheet shows money by its column type, formulas worked out, and a bad one as written', () => {
    const html = draw(`---\nsuite_kind: sheet\n---\n${SHEET}`)
    expect(html).toContain('UGX 4,200')
    expect(html).toContain('UGX 168,000')
    expect(html).toContain('=NOPE(1)')
    expect(html).toContain('su-bad')
  })
  test('a dashboard tile that cannot be worked out says why instead of showing a zero', () => {
    const html = draw(`---\nsuite_kind: dashboard\n---\n${SHEET}\n\`\`\`tile from=[[Other]]!A1:A3 fn=sum label="Elsewhere"\n\`\`\`\n`)
    expect(html).toContain('Reads another note')
    expect(html).not.toContain('su-big')
  })
  test('slides are cards with a strip, one stage and a step control', () => {
    const html = draw('---\nsuite_kind: slides\n---\n# Deck\n\n## One\n\nHello\n\n## Two\n\nWorld\n')
    expect(html).toContain('su-strip')
    expect(html).toContain('Slide 1 of 2')
    expect(html).toContain('Next slide')
  })
  test('a note that is not a sheet, slides or dashboard draws nothing here', () => {
    expect(draw('---\nsuite_kind: doc\n---\n# Doc\n')).not.toContain('su-sheet')
  })
})

describe('editing in the viewers', () => {
  test('without a place to save to, nothing can be edited', () => {
    const html = draw('---\nsuite_kind: slides\n---\n# D\n\n## One\n\nHi\n')
    expect(html).not.toContain('su-title-input')
    expect(html).not.toContain('Edit text')
  })
  test('with one, a card title is a field, text has an Edit button, and a sheet can take a row', () => {
    const slides = renderToStaticMarkup(createElement(SuiteViewer, { text: '---\nsuite_kind: slides\n---\n# D\n\n## One\n\nHi\n', path: 'Notebooks/D.md', hash: 'h' }))
    expect(slides).toContain('su-title-input')
    expect(slides).toContain('Edit text')
    const sheet = renderToStaticMarkup(createElement(SuiteViewer, { text: `---\nsuite_kind: sheet\n---\n${SHEET}`, path: 'Notebooks/S.md', hash: 'h' }))
    expect(sheet).toContain('Add a row')
  })
  test('a file kept view-only is never offered an edit', () => {
    const html = renderToStaticMarkup(createElement(SuiteViewer, { text: `---\nsuite_kind: sheet\nsuite_fidelity: view\n---\n${SHEET}`, path: 'Notebooks/S.md', hash: 'h' }))
    expect(html).not.toContain('Add a row')
    expect(html).toContain('cannot be changed here')
  })
})
