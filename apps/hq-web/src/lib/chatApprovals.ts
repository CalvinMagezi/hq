import type { NotificationItem } from './notificationsApi'

const SAID = 'You said:'

/** The approval items that belong to one web chat, oldest first. Other chat apps carry no thread id. */
export function approvalsForThread(items: NotificationItem[], threadId: string | null): NotificationItem[] {
  if (!threadId) return []
  return items
    .filter((i) => i.metadata.approval?.thread_id === threadId)
    .sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at))
}

/** A voice turn's request starts "You said: “…”"; the words heard and the rest of HQ's description, apart. */
export function splitSaid(description: string): { said: string | null; rest: string } {
  const text = description.trim()
  if (!text.startsWith(SAID)) return { said: null, rest: text }
  const [first, ...more] = text.split('\n')
  return { said: first.trim(), rest: more.join('\n').trim() }
}

export type ApprovalView = 'pending' | 'answered_here' | 'answered_elsewhere'

/** Where an item stands for this chat: waiting, answered from this chat, or answered from the Inbox or another device. */
export function approvalView(item: Pick<NotificationItem, 'id' | 'state'>, answeredHere: ReadonlySet<string>): ApprovalView {
  if (item.state === 'pending') return 'pending'
  return answeredHere.has(item.id) ? 'answered_here' : 'answered_elsewhere'
}
