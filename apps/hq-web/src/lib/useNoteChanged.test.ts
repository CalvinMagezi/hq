import { expect, test } from 'bun:test'
import { QueryClient } from '@tanstack/react-query'
import { NOTE_CHANGED, invalidateNote, noteChangedIn } from './useNoteChanged'
import { CHANGE_EVENTS, sharedWithSubscribers } from '~/context/WebSocketContext'

test('a note_changed frame names the note and its new revision, and no other frame does', () => {
  expect(noteChangedIn({ type: NOTE_CHANGED, path: 'Notebooks/a.md', hash: 'abc123' })).toEqual({ path: 'Notebooks/a.md', hash: 'abc123' })
  expect(noteChangedIn({ type: NOTE_CHANGED, path: 'Notebooks/a.md' })).toEqual({ path: 'Notebooks/a.md', hash: '' })
  expect(noteChangedIn({ type: 'notes:change' })).toBeNull()
  expect(noteChangedIn({ type: NOTE_CHANGED })).toBeNull()
  expect(noteChangedIn({ type: NOTE_CHANGED, path: '' })).toBeNull()
  expect(noteChangedIn({ type: NOTE_CHANGED, path: 7 })).toBeNull()
})

test('subscribers hear it even when chat handling consumed the frame', () => {
  expect(CHANGE_EVENTS.has(NOTE_CHANGED)).toBe(true)
  expect(sharedWithSubscribers(NOTE_CHANGED)).toBe(true)
})

test('only the reads of the changed note go stale', async () => {
  const client = new QueryClient()
  const keys = [
    ['pane-note', 'Notebooks/a.md'],
    ['note-versions', 'Notebooks/a.md'],
    ['suite-cards', 'Notebooks/a.md'],
    ['pane-note', 'Notebooks/b.md'],
    ['search', 'tree'],
  ]
  for (const key of keys) client.setQueryData(key, { ok: true })
  await invalidateNote(client, 'Notebooks/a.md')
  const stale = keys.map((key) => client.getQueryState(key)?.isInvalidated)
  expect(stale).toEqual([true, true, true, false, false])
})
