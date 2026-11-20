import type { NotificationItem } from './notificationsApi'
import { isSandboxGrantNotification, parseSandboxGrantNotification } from './sandboxGrantsApi'

const SAID = 'You said:'

/** The approval items that belong to one web chat, oldest first. Includes sandbox-grants requests for sessions this chat watches. */
export function approvalsForThread(
  items: NotificationItem[],
  threadId: string | null,
  sessionIds: readonly string[] = [],
): NotificationItem[] {
  if (!threadId) return []
  const watched = new Set(sessionIds)
  return items
    .filter((i) => {
      if (i.metadata.approval?.thread_id === threadId) return true
      if (watched.size > 0 && isSandboxGrantNotification(i)) {
        const sid = parseSandboxGrantNotification(i).sessionId
        return sid !== null && watched.has(sid)
      }
      return false
    })
    .sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at))
}

/** A voice turn's request starts "You said: “…”"; the words heard and the rest of HQ's description, apart. */
export function splitSaid(description: string): { said: string | null; rest: string } {
  const text = description.trim()
  if (!text.startsWith(SAID)) return { said: null, rest: text }
  const [first, ...more] = text.split('\n')
  return { said: first.trim(), rest: more.join('\n').trim() }
}
