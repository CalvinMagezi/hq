// Vault reads as React Query queries, so they persist on the device and paint
// instantly on the next visit.
import { queryOptions } from '@tanstack/react-query'
import { getNote, getNoteTree, getPinnedNotes, getVaultSignals } from './vaultApi'

export const vaultKeys = {
  all: ['vault'] as const,
  signals: ['vault', 'signals'] as const,
  pinned: ['vault', 'pinned'] as const,
  tree: (root: string) => ['vault', 'tree', root] as const,
  note: (path: string) => ['vault', 'note', path] as const,
}

export const vaultSignalsQuery = queryOptions({ queryKey: vaultKeys.signals, queryFn: getVaultSignals })
export const pinnedNotesQuery = queryOptions({ queryKey: vaultKeys.pinned, queryFn: getPinnedNotes })
export const noteTreeQuery = (root: string) =>
  queryOptions({ queryKey: vaultKeys.tree(root), queryFn: () => getNoteTree(root) })
export const noteQuery = (path: string) =>
  queryOptions({ queryKey: vaultKeys.note(path), queryFn: () => getNote(path) })
