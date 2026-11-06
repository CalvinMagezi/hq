import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { Panes, PaneHead, panesClass } from './Panes'
import { Sheet } from './Sheet'

const read = (p: string) => readFileSync(join(import.meta.dir, '..', '..', '..', p), 'utf8')

// web-matches-phone wave A items 4 and 5: one layout for a list and its open item, one sheet.
describe('panes (AlignedShell 2)', () => {
  test('the class list follows what the place has and what is open', () => {
    expect(panesClass({ list: 'x', detail: 'y' } as never)).toBe('a-panes hq-panes')
    expect(panesClass({ list: undefined } as never)).toBe('a-panes hq-panes no-list')
    expect(panesClass({ list: 'x', side: 'z', showDetail: true, sideOpen: true } as never)).toBe('a-panes hq-panes has-side is-detail side-open')
  })
  test('list, detail and details render as the design names them', () => {
    const html = renderToStaticMarkup(
      createElement(Panes, {
        list: 'L', detail: createElement(PaneHead, { title: 'Plan' }), side: 'S',
        listLabel: 'Chats', detailLabel: 'Plan', sideLabel: 'Details',
      }),
    )
    expect(html).toContain('<section class="a-pane a-pane-list" aria-label="Chats">L</section>')
    expect(html).toContain('<div class="a-pane-head"><h2 class="a-h3">Plan</h2></div>')
    expect(html).toContain('<aside class="a-pane a-pane-side" aria-label="Details">S</aside>')
  })
  test('details lie over the open item below 1400 px and dock from 1400', () => {
    const css = read('app.css')
    expect(css).toMatch(/@container app \(max-width:1399px\)\{\s*\.hq-panes\.side-open \.a-pane-side\{display:flex;position:absolute/)
    expect(read('src/styles/aligned-ui.css')).toContain('.a-panes.has-side .a-pane-side{display:flex')
  })
})

describe('the sheet (AlignedFoundations 2)', () => {
  test('a modal dialog with the handle, a titled head, a close button, the body and the foot', () => {
    const html = renderToStaticMarkup(
      createElement(Sheet, { title: 'Export as', onClose: () => undefined, placement: 'side', footer: createElement('button', null, 'Save') }, 'Body'),
    )
    expect(html).toMatch(/^<dialog class="a-sheet side hq-sheet" aria-labelledby="[^"]+">/)
    expect(html).toContain('<span class="a-handle" aria-hidden="true"></span>')
    expect(html).toMatch(/<h2 class="a-h2" id="[^"]+">Export as<\/h2>/)
    expect(html).toContain('aria-label="Close"')
    expect(html).toContain('<div class="a-sheet-body">Body</div><div class="a-sheet-foot"><button>Save</button></div>')
  })
  test('the checking sheets use it, with the design buttons, not a hand-made overlay', () => {
    const src = read('src/components/tasks/verification/Sheet.tsx')
    expect(src).toContain("from '~/components/ui/Sheet'")
    expect(src).not.toMatch(/createPortal|hq-modal|emerald|rose/)
  })
})
