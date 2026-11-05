// One Search: matching tasks, helpers and chats in this browser, the recent list, and the failure line
// (docs/plans/ux-run/web/wave-4/04-search.md). Chats are matched from the thread list saved in this
// browser: HQ has no chat search route.

import { t } from '~/strings'
import type { ConnectionState } from './connection'
import type { SearchHit } from './vaultApi'
import type { TaskItem } from './tasksApi'
import type { HarnessSession } from './sessionsApi'
import type { Thread } from '~/store/threadStore'
import { agentName, computerName } from './names'
import { sessionTitle } from './workbench'

/** Search waits this long after the last key (WR12). */
export const SEARCH_DEBOUNCE_MS = 300
/** Rows a group shows before "Show all N". */
export const GROUP_SIZE = 5
export const RECENT_MAX = 8
/** Notes the Ask HQ button attaches at most. */
export const ASK_MAX = 10

export type SearchScope = 'all' | 'notes' | 'tasks' | 'chats' | 'helpers'
export const SCOPES: SearchScope[] = ['all', 'notes', 'tasks', 'chats', 'helpers']
/** Lite has no chats or helpers. */
export const LITE_SCOPES: SearchScope[] = ['all', 'notes', 'tasks']

const normalize = (s: string) => s.toLowerCase()

function matches(q: string, ...fields: (string | null | undefined)[]): boolean {
  const words = normalize(q).split(/\s+/).filter(Boolean)
  if (words.length === 0) return false
  const hay = normalize(fields.filter(Boolean).join(' '))
  return words.every((w) => hay.includes(w))
}

export function matchTasks(tasks: TaskItem[], q: string): TaskItem[] {
  return tasks.filter((x) => !x.archived_at && matches(q, x.title, x.display_id, x.description, x.tags.join(' ')))
}

/** Helpers are found by job, helper name and computer. */
export function matchHelpers(sessions: HarnessSession[], q: string): HarnessSession[] {
  return sessions.filter(
    (s) => !s.archived && matches(q, sessionTitle(s), s.task?.title, agentName(s.harness), s.agent_name, computerName(s.host)),
  )
}

export function matchChats(threads: Thread[], q: string): Thread[] {
  return threads.filter((x) => x.status === 'active' && matches(q, x.title, x.lastMessagePreview))
}

export interface FoundPart {
  text: string
  found: boolean
}

/** Splits text so the words found can be drawn bold, never washed in a colour (A3). */
export function foundParts(text: string, q: string): FoundPart[] {
  const words = [...new Set(q.trim().split(/\s+/).filter(Boolean))]
  if (words.length === 0 || !text) return [{ text, found: false }]
  const escaped = words.sort((a, b) => b.length - a.length).map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
  const re = new RegExp(`(${escaped.join('|')})`, 'gi')
  // With one capture group, split puts every match at an odd index.
  return text
    .split(re)
    .map((p, i) => ({ text: p, found: i % 2 === 1 }))
    .filter((p) => p.text !== '')
}

/** The failure line for notes that could not be searched, by what the connection says. */
export function searchFailureLine(state: ConnectionState, lite: boolean): string {
  if (lite && state !== 'offline') return t('search_lite_fail')
  switch (state) {
    case 'starting':
      return t('fail_search_starting')
    case 'offline':
      return t('fail_search_offline')
    case 'connected':
    case 'ai_down':
      return t('fail_search_no_reply')
    default:
      return t('fail_search_unreachable')
  }
}

/** The top folders of the hits, with counts: the folder chips. */
export function topFolders(hits: SearchHit[]): { folder: string; count: number }[] {
  const counts = new Map<string, number>()
  for (const h of hits) {
    const top = h.notePath.split('/')[0]
    if (h.notePath.includes('/') && top) counts.set(top, (counts.get(top) ?? 0) + 1)
  }
  return [...counts.entries()].map(([folder, count]) => ({ folder, count })).sort((a, b) => b.count - a.count || a.folder.localeCompare(b.folder))
}

/** The last path part, for "In Agent-HQ". */
export const folderLabel = (folder: string) => folder.split('/').filter(Boolean).pop() ?? folder

/** A folder path as the trail "Projects › Agent-HQ". */
export const folderTrail = (folder: string) => folder.split('/').filter(Boolean).join(' › ')

/** Rows shown for a group: five, or all once the group was chosen. */
export function visibleRows<T>(rows: T[], all: boolean): T[] {
  return all ? rows : rows.slice(0, GROUP_SIZE)
}

const recentKey = (hqId: string | null) => `hq-search-recent:${hqId ?? 'none'}`

export function readRecent(hqId: string | null, store: Storage = localStorage): string[] {
  try {
    const parsed: unknown = JSON.parse(store.getItem(recentKey(hqId)) ?? '[]')
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === 'string').slice(0, RECENT_MAX) : []
  } catch {
    return []
  }
}

export function writeRecent(hqId: string | null, list: string[], store: Storage = localStorage): string[] {
  const next = list.slice(0, RECENT_MAX)
  try {
    store.setItem(recentKey(hqId), JSON.stringify(next))
  } catch {
    // Blocked storage: the list lives for this page only.
  }
  return next
}

/** A query that led to a result goes first; eight at most. */
export function withRecent(list: string[], query: string): string[] {
  const q = query.trim()
  if (!q) return list
  return [q, ...list.filter((x) => x.toLowerCase() !== q.toLowerCase())].slice(0, RECENT_MAX)
}
