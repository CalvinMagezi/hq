import { create } from 'zustand'

export interface VaultNoteState {
  notePath: string | null
  open: boolean
  triggerElement: HTMLElement | null
  openNote: (rawPath: string, trigger?: HTMLElement | null) => boolean
  closeNote: () => void
}

/**
 * Sanitizes a vault note path reference, stripping wikilink syntax, protocol prefixes,
 * and rejecting external URLs, protocols, and directory traversal.
 * Returns the clean vault-relative path, or null if invalid/unsafe.
 */
export function sanitizeVaultPath(raw: string | undefined | null): string | null {
  if (!raw) return null
  let p = raw.trim()

  // Handle Wikilinks: [[Note Name]] or [[path/to/Note|Label]]
  if (p.startsWith('[[') && p.endsWith(']]')) {
    p = p.slice(2, -2).trim()
  }
  // Strip alias if present
  if (p.includes('|')) {
    p = p.split('|')[0].trim()
  }

  // Handle custom schemes
  if (p.toLowerCase().startsWith('vault:')) {
    p = p.slice(6).trim()
  } else if (p.toLowerCase().startsWith('note:')) {
    p = p.slice(5).trim()
  }

  // Handle /api/note?path=...
  if (p.startsWith('/api/note')) {
    try {
      const qIdx = p.indexOf('?')
      if (qIdx >= 0) {
        const params = new URLSearchParams(p.slice(qIdx))
        p = params.get('path') ?? ''
      }
    } catch {
      return null
    }
  }

  p = p.trim()
  if (!p) return null

  // Security checks:
  // 1. Must not be external protocol / link
  const lower = p.toLowerCase()
  if (
    lower.startsWith('http://') ||
    lower.startsWith('https://') ||
    lower.startsWith('//') ||
    lower.startsWith('mailto:') ||
    lower.startsWith('javascript:') ||
    lower.startsWith('data:')
  ) {
    return null
  }

  // 2. Reject absolute paths or directory traversal
  if (p.startsWith('/') || p.startsWith('\\') || p.includes('..')) {
    return null
  }

  return p
}

export const useVaultNoteStore = create<VaultNoteState>()((set) => ({
  notePath: null,
  open: false,
  triggerElement: null,

  openNote: (rawPath, trigger) => {
    const clean = sanitizeVaultPath(rawPath)
    if (!clean) return false
    set({
      notePath: clean,
      open: true,
      triggerElement: trigger ?? (typeof document !== 'undefined' ? (document.activeElement as HTMLElement) : null),
    })
    return true
  },

  closeNote: () => {
    set((s) => {
      if (s.triggerElement && typeof s.triggerElement.focus === 'function') {
        setTimeout(() => s.triggerElement?.focus(), 0)
      }
      return { notePath: null, open: false, triggerElement: null }
    })
  },
}))
