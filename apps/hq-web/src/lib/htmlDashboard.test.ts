import { describe, expect, test } from 'bun:test'
import { extractHtmlDocument, htmlSaveFileName, isHtmlPath } from './htmlDashboard'

describe('htmlDashboard', () => {
  test('isHtmlPath matches .html and .htm paths', () => {
    expect(isHtmlPath('Exports/2026-11/dash.html')).toBe(true)
    expect(isHtmlPath('Notebooks/Report.HTM')).toBe(true)
    expect(isHtmlPath('Notebooks/Plan.md')).toBe(false)
    expect(isHtmlPath(null)).toBe(false)
  })

  test('extractHtmlDocument detects standalone HTML in a markdown note (with or without frontmatter)', () => {
    const doc = '<!DOCTYPE html><html><body><h1>Sales</h1></body></html>'
    expect(extractHtmlDocument(doc, 'Notebooks/Sales.md')).toBe(doc)

    const withFm = `---\ntitle: Sales Dashboard\n---\n\n<html lang="en"><body><script>console.log(1)</script></body></html>`
    expect(extractHtmlDocument(withFm, 'Notebooks/Sales.md')).toBe(
      '<html lang="en"><body><script>console.log(1)</script></body></html>',
    )
  })

  test('extractHtmlDocument accepts any non-empty .html file and rejects ordinary markdown', () => {
    expect(extractHtmlDocument('<div>Widget</div>', 'Exports/dash.html')).toBe('<div>Widget</div>')
    expect(extractHtmlDocument('# Heading\n\n| A | B |\n|---|---|\n| 1 | 2 |', 'Notebooks/Sheet.md')).toBeNull()
    expect(extractHtmlDocument('Here is an example:\n\n```html\n<html></html>\n```', 'Notebooks/Guide.md')).toBeNull()
  })

  test('htmlSaveFileName produces a .html file name', () => {
    expect(htmlSaveFileName('Exports/2026-11/dash.html')).toBe('dash.html')
    expect(htmlSaveFileName('Notebooks/Sales Dashboard.md')).toBe('Sales Dashboard.html')
    expect(htmlSaveFileName(null)).toBe('dashboard.html')
  })
})
