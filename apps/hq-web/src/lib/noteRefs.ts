import { sanitizeVaultPath } from '~/store/vaultNoteStore'

const MAX_NOTE_REF_LENGTH = 300
// Characters a vault path never has but prose and shell snippets in backticks often do.
const NOT_A_PATH = /[\r\n\t`<>*?"|]/

function escapeAttr(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

function safeDecode(text: string): string {
  try {
    return decodeURIComponent(text)
  } catch {
    return text
  }
}

/**
 * The vault path a backticked chat reference points at, or null when it is just code.
 * Agents name notes as inline code (`Notebooks/Projects/Plan.md`) far more often than as
 * links, so a path-shaped `.md` reference is treated as one. It needs a folder in it: a bare
 * `README.md` is nearly always talking about a repo file, not a note.
 */
export function noteRefFromCode(text: string | null | undefined): string | null {
  if (!text || text !== text.trim() || text.length > MAX_NOTE_REF_LENGTH) return null
  if (NOT_A_PATH.test(text) || !text.includes('/') || !/\.md$/i.test(text)) return null
  return sanitizeVaultPath(text)
}

/**
 * The vault path a markdown link points at, or null when it is an ordinary link.
 * Handles `vault:` and `note:` links, `/api/note?path=` links and relative `.md` links,
 * including the percent-encoded form a path with spaces takes and a trailing `#heading`.
 */
export function vaultLinkTarget(href: string | null | undefined): string | null {
  if (!href) return null
  const h = href.trim()
  if (h.startsWith('/api/note')) return sanitizeVaultPath(h)
  const withoutFragment = h.split('#')[0]
  const isScheme = /^(vault|note):/i.test(withoutFragment)
  const isMarkdownFile = !h.includes('://') && /\.md$/i.test(withoutFragment)
  if (!isScheme && !isMarkdownFile) return null
  return sanitizeVaultPath(safeDecode(withoutFragment))
}

/** Inline code as HTML: a tappable note reference when it names a vault note, plain `<code>` otherwise. */
export function codespanHtml(text: string): string {
  const escaped = escapeAttr(text)
  const note = noteRefFromCode(text)
  if (!note) return `<code>${escaped}</code>`
  const path = escapeAttr(note)
  return `<code class="md-note-ref" role="button" tabindex="0" data-vault-note="${path}" title="Open note" aria-label="Open note ${path}">${escaped}</code>`
}
