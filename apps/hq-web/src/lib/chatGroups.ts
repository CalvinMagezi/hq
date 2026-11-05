import { t } from '~/strings'

type Key = Parameters<typeof t>[0]

export interface ChatGroup<T> {
  key: 'today' | 'yesterday' | 'week' | 'older'
  label: Key
  items: T[]
}

const DAY_MS = 86_400_000

/** Groups a newest-first chat list by the day it last moved: Today, Yesterday, Previous 7 days, Older. Empty groups are left out. */
export function groupChats<T extends { updatedAt: number }>(chats: T[], now: number = Date.now()): ChatGroup<T>[] {
  const startOfToday = new Date(now)
  startOfToday.setHours(0, 0, 0, 0)
  const today = startOfToday.getTime()
  const groups: ChatGroup<T>[] = [
    { key: 'today', label: 'chat_group_today', items: [] },
    { key: 'yesterday', label: 'chat_group_yesterday', items: [] },
    { key: 'week', label: 'chat_group_week', items: [] },
    { key: 'older', label: 'chat_group_older', items: [] },
  ]
  for (const c of chats) {
    const i = c.updatedAt >= today ? 0 : c.updatedAt >= today - DAY_MS ? 1 : c.updatedAt >= today - 7 * DAY_MS ? 2 : 3
    groups[i].items.push(c)
  }
  return groups.filter((g) => g.items.length > 0)
}
