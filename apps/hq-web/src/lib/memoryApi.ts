// What HQ remembers: the owner's list of the short notes HQ saves from their chats (`/api/memory`).
// Only the owner's own apps may call these routes. Nothing here sends a memory anywhere else.

import { hqJson } from './hqAuth'
import type { StringKey } from '~/strings'

export type MemoryStatus = 'active' | 'review' | 'superseded' | string
export type RecallMode = 'shadow' | 'query' | 'recent' | 'off'
export type MemoryFilter = 'all' | 'review' | 'outdated' | 'unattributed' | 'pinned'
export type Standing = 'active' | 'review' | 'outdated'

export interface MemoryItem {
  id: number
  summary: string
  entities: string[]
  topics: string[]
  importance: number
  created_at: string
  status: MemoryStatus
  /** `owner`, or `unknown_channel` for a note from a shared channel HQ cannot tie to the owner. */
  origin: 'owner' | 'unknown_channel' | string
  provenance: 'owner_said' | 'model_inferred' | 'owner_edited' | string
  pinned: boolean
  used: number
  thread_id: string | null
  harness: string
}

export interface MemoryCounts {
  total: number
  active: number
  review: number
  out_of_date: number
  unattributed: number
  pinned: number
}

export interface MemoryList {
  counts: MemoryCounts
  items: MemoryItem[]
  recall: RecallMode | null
}

export interface ForgetResult {
  success: boolean
  report: {
    insights_removed: number
    insight_notes_removed: string[]
    concept_pages_updated: number
    concept_pages_removed: number
    archive_entries_removed: number
  }
  not_reached: string[]
}

/** Whether a change worked. */
export type Done = Promise<boolean>
/** A change that is still running. */
export type Pending = Promise<unknown>

export const MEMORY_QUERY_KEY = ['memory'] as const

export const MEMORY_FILTERS: { id: MemoryFilter; label: StringKey }[] = [
  { id: 'all', label: 'memory_filter_all' },
  { id: 'review', label: 'memory_filter_review' },
  { id: 'outdated', label: 'memory_filter_outdated' },
  { id: 'unattributed', label: 'memory_filter_unattributed' },
  { id: 'pinned', label: 'memory_filter_pinned' },
]

export function listMemories(filter: MemoryFilter, q: string): Promise<MemoryList> {
  const params = new URLSearchParams()
  if (q.trim()) params.set('q', q.trim())
  else if (filter !== 'all') params.set('filter', filter)
  const query = params.toString()
  return hqJson<MemoryList>(`/api/memory${query ? `?${query}` : ''}`)
}

export function editMemory(id: number, summary: string): Promise<{ success: boolean; item: MemoryItem }> {
  return hqJson(`/api/memory/${id}/edit`, 'POST', { summary })
}

export function pinMemory(id: number, pinned: boolean): Promise<{ success: boolean; item: MemoryItem }> {
  return hqJson(`/api/memory/${id}/pin`, 'POST', { pinned })
}

export function setMemoryStanding(id: number, standing: Standing): Promise<{ success: boolean; item: MemoryItem }> {
  return hqJson(`/api/memory/${id}/standing`, 'POST', { standing })
}

export function forgetMemory(id: number): Promise<ForgetResult> {
  return hqJson(`/api/memory/${id}/forget`, 'POST')
}

/** The words that say what is unusual about a memory, in the order they are shown. Nothing for an ordinary one. */
export function memoryBadges(item: MemoryItem): StringKey[] {
  const badges: StringKey[] = []
  if (item.pinned) badges.push('memory_badge_pinned')
  if (item.status === 'review') badges.push('memory_badge_review')
  if (item.status === 'superseded') badges.push('memory_badge_outdated')
  if (item.origin !== 'owner') badges.push('memory_badge_shared')
  if (item.provenance === 'owner_edited') badges.push('memory_badge_edited')
  return badges
}

/** The line that says how memories reach a chat right now. */
export function recallModeKey(mode: RecallMode | null): StringKey | null {
  switch (mode) {
    case 'shadow':
      return 'memory_mode_shadow'
    case 'query':
      return 'memory_mode_query'
    case 'recent':
      return 'memory_mode_recent'
    case 'off':
      return 'memory_mode_off'
    default:
      return null
  }
}

/** Whether a memory is held out of chats until the owner looks at it. */
export function isHeldBack(item: MemoryItem): boolean {
  return item.status === 'review' || item.origin !== 'owner'
}

/** What the one standing button of a note does: keep a held-back one, bring an outdated one back, or retire a live one. */
export function standingAction(item: MemoryItem): { label: StringKey; to: Standing } {
  if (isHeldBack(item)) return { label: 'memory_keep', to: 'active' }
  if (item.status === 'superseded') return { label: 'memory_use_again', to: 'active' }
  return { label: 'memory_mark_outdated', to: 'outdated' }
}
