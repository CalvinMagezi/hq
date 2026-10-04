export const BADGE_CAP = 99

export interface InboxBadge {
  label: string
  aria: string
}

/** Null when there is nothing to show, so callers render no badge at all. */
export function inboxBadge(count: number): InboxBadge | null {
  const n = Math.floor(count)
  if (!Number.isFinite(n) || n < 1) return null
  return {
    label: n > BADGE_CAP ? `${BADGE_CAP}+` : String(n),
    aria: `${n} unread ${n === 1 ? 'notification' : 'notifications'}`,
  }
}
