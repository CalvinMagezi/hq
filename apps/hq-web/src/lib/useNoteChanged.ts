import { useEffect } from 'react'
import { useQueryClient, type QueryClient } from '@tanstack/react-query'
import { useWSOptional } from '~/context/WebSocketContext'

/** The socket event hq sends when a note was written by an agent, another device or a suite edit. */
export const NOTE_CHANGED = 'note_changed'

export interface NoteChanged {
  /** The note's vault path. */
  path: string
  /** The note's revision after the write. */
  hash: string
}

/** The note a socket frame says changed, or null for any other frame. */
export function noteChangedIn(msg: Record<string, unknown>): NoteChanged | null {
  if (msg.type !== NOTE_CHANGED || typeof msg.path !== 'string' || !msg.path) return null
  return { path: msg.path, hash: typeof msg.hash === 'string' ? msg.hash : '' }
}

/**
 * Marks every cached read of the note at `path` stale, so the one on screen refetches. A query
 * belongs to the note when its key holds the path (`['pane-note', path]`, `['note-versions', path]`,
 * `['suite-cards', path]`); notes that are not open have no active query and cost nothing.
 */
export function invalidateNote(client: QueryClient, path: string): Promise<void> {
  return client.invalidateQueries({ predicate: (q) => q.queryKey.includes(path) })
}

/** Calls `onChange` when the note at `path` changed somewhere else. */
export function useNoteChanged(path: string | null | undefined, onChange: (change: NoteChanged) => void): void {
  const ws = useWSOptional()
  useEffect(() => {
    if (!ws || !path) return
    return ws.subscribe((msg) => {
      const change = noteChangedIn(msg)
      if (change && change.path === path) onChange(change)
    })
  }, [ws, path, onChange])
}

/** Reloads any note on screen when hq says it changed. Mounted once, under the socket and the query client. */
export function NoteChangedBridge(): null {
  const ws = useWSOptional()
  const client = useQueryClient()
  useEffect(() => {
    if (!ws) return
    return ws.subscribe((msg) => {
      const change = noteChangedIn(msg)
      if (change) void invalidateNote(client, change.path)
    })
  }, [ws, client])
  return null
}
