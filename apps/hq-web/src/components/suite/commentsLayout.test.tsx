import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { SuiteViewer } from './SuiteViewer'
import { DocBody } from './DocTools'

const SALES = '```sheet name=Sales types="text,int"\n| month | amount |\n|---|---|\n| Jan | 10 |\n| Feb | 20 |\n| Mar | 30 |\n```\n'
const TILE = '```tile from=B2:B4 fn=sum label="Total"\n```\n'
const PLOT = '```plot type=bar from=A1:B4 x=month y=amount title="By month" size=full\n```\n'
const COMMENT = '```comment on=0123456789ab by="Sam" at=2026-11-20 state=open\nCheck this.\n```\n'
const draw = (text: string, path?: string) => renderToStaticMarkup(createElement(SuiteViewer, { text, path, hash: 'h' }))
const DASH = `---\nsuite_kind: dashboard\n---\n# Sales\n\n${SALES}\n${PLOT}\n${TILE}`

describe('the dashboard layout', () => {
  test('tiles and plots come in the order the note has them, each with the size it asks for', () => {
    const html = draw(DASH)
    expect(html.indexOf('su-plot')).toBeLessThan(html.indexOf('su-tile'))
    expect(html).toContain('su-slot su-size-full')
    expect(html).toContain('su-slot su-size-small')
  })
  test('a plot that names no size is wide', () => {
    expect(draw(`---\nsuite_kind: dashboard\n---\n${SALES}\n${PLOT.replace(' size=full', '')}`)).toContain('su-size-wide')
  })
  test('Arrange is offered only where the note can be changed', () => {
    expect(draw(DASH)).not.toContain('Arrange')
    expect(draw(DASH, 'Notebooks/D.md')).toContain('Arrange')
    expect(draw(DASH.replace('suite_kind: dashboard', 'suite_kind: dashboard\nsuite_fidelity: view'), 'Notebooks/D.md')).not.toContain('Arrange')
  })
  test('nothing is being arranged until the person asks: no bars, no hint', () => {
    const html = draw(DASH, 'Notebooks/D.md')
    expect(html).not.toContain('su-slot-bar')
    expect(html).not.toContain('Move a block')
  })
})

describe('the comments button', () => {
  const DECK = `---\nsuite_kind: slides\n---\n# D\n\n## One\n\nHi\n\n${COMMENT}\n${COMMENT.replace('open', 'resolved')}`
  test('is there with a place to save to, with the count of open comments', () => {
    const html = draw(DECK, 'Notebooks/D.md')
    expect(html).toContain('>Comments<')
    expect(html).toContain('aria-label="Comments, 1 open"')
    expect(html).toContain('su-badge')
  })
  test('has no badge when nothing is open, and is not offered with no place to read from', () => {
    const none = draw('---\nsuite_kind: slides\n---\n# D\n\n## One\n\nHi\n', 'Notebooks/D.md')
    expect(none).toContain('aria-label="Comments"')
    expect(none).not.toContain('su-badge')
    expect(draw(DECK)).not.toContain('Comments')
  })
  test('is on a sheet, a dashboard and a doc as well', () => {
    expect(draw(`---\nsuite_kind: sheet\n---\n${SALES}`, 'Notebooks/S.md')).toContain('>Comments<')
    expect(draw(DASH, 'Notebooks/D.md')).toContain('>Comments<')
    const doc = renderToStaticMarkup(createElement(DocBody, { content: `---\nsuite_kind: doc\n---\n# T\n\nHello.\n\n${COMMENT}`, path: 'Notebooks/R.md', hash: 'h' }))
    expect(doc).toContain('>Comments<')
    expect(doc).toContain('aria-label="Comments, 1 open"')
  })
  test('a comment block is not a sheet, a tile or a plot, so no viewer draws it', () => {
    const html = draw(`---\nsuite_kind: dashboard\n---\n${SALES}\n${TILE}\n${COMMENT}`, 'Notebooks/D.md')
    expect(html).not.toContain('Check this.')
    expect(html.match(/class="su-tile"|su-tile"/g)).toHaveLength(1)
  })
})
