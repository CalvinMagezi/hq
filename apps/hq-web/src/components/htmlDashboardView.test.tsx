import { describe, expect, test } from 'bun:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { HtmlDashboardView } from './HtmlDashboardView'

describe('HtmlDashboardView', () => {
  test('renders a sandboxed iframe with allow-scripts and without allow-same-origin, plus full screen and save actions', () => {
    const html = renderToStaticMarkup(
      createElement(HtmlDashboardView, {
        html: '<!DOCTYPE html><html><body><h1>Sales</h1></body></html>',
        path: 'Exports/2026-11/dash.html',
        title: 'Sales Dashboard',
      }),
    )
    expect(html).toContain('<iframe')
    expect(html).toContain('sandbox="allow-scripts"')
    expect(html).not.toContain('allow-same-origin')
    expect(html).toContain('srcDoc=')
    expect(html).toContain('Open full screen')
    expect(html).toContain('Save as file')
  })
})
