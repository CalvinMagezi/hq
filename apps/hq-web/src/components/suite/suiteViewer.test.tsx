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
