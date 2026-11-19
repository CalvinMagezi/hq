import { splitFrontmatter } from './hqmd'

const HTML_EXT = /\.html?$/i
const HTML_DOC_START = /^(?:<!doctype\s+html\b|<html\b)/i

/** Whether a vault path points at an HTML document. */
export function isHtmlPath(path: string | null | undefined): boolean {
  return typeof path === 'string' && HTML_EXT.test(path.trim())
}

/**
 * Returns the standalone HTML document when a note or file is an HTML dashboard,
 * or null when it should render as normal markdown or a suite note.
 */
export function extractHtmlDocument(content: string, path?: string | null): string | null {
  if (!content) return null
  const { body } = splitFrontmatter(content)
  const trimmed = body.trim()
  if (!trimmed) return null
  if (isHtmlPath(path)) return trimmed
  if (HTML_DOC_START.test(trimmed)) return trimmed
  return null
}

/** File name used when saving an HTML dashboard to disk. */
export function htmlSaveFileName(path?: string | null): string {
  const base = path?.split('/').filter(Boolean).pop()?.trim()
  if (!base) return 'dashboard.html'
  if (HTML_EXT.test(base)) return base
  if (/\.md$/i.test(base)) return base.replace(/\.md$/i, '.html')
  return `${base}.html`
}
