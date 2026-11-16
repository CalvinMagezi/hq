import { hqJson } from './hqAuth'

/** Where one office file stands for the HQ Suite (`GET /api/suite/file`); hq writes every sentence. */
export interface SuiteFile {
  path: string
  state: 'ready' | 'prepared' | 'shrunk'
  job: string | null
  note: string | null
  saved_bytes: number | null
  headline: string
  details: string[]
  actions: { shrink: boolean; confirm: boolean; abandon: boolean; undo: boolean }
}

export const fetchSuiteFile = (path: string) => hqJson<SuiteFile>(`/api/suite/file?path=${encodeURIComponent(path)}`)
export const suiteShrink = (path: string) => hqJson<SuiteFile>('/api/suite/shrink', 'POST', { path })
export const suiteConfirm = (job: string) => hqJson<SuiteFile>('/api/suite/confirm', 'POST', { job })
export const suiteAbandon = (job: string) => hqJson<SuiteFile>('/api/suite/abandon', 'POST', { job })
export const suiteUndo = (job: string) => hqJson<SuiteFile>('/api/suite/undo', 'POST', { job })

/**
 * The office file a note was made from, read from its front matter: the note names a `source` and the
 * hash of what it was made from, and does not say its original is gone. Null for any other note.
 */
export function suiteSourceOf(text: string): string | null {
  const lines = text.split('\n')
  if (lines[0]?.trim() !== '---') return null
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === '---')
  if (end < 0) return null
  const keys = new Map<string, string>()
  for (const line of lines.slice(1, end)) {
    const at = line.indexOf(':')
    if (at <= 0 || /^\s/.test(line)) continue
    keys.set(line.slice(0, at).trim(), line.slice(at + 1).trim().replace(/^["']|["']$/g, ''))
  }
  const source = keys.get('source')
  if (!source || !keys.get('source_sha256')) return null
  return keys.get('suite_original_state') === 'gone' ? null : source
}
