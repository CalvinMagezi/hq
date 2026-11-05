// Folding pushes so a busy HQ never buries the lock screen (docs/plans/ux-run/web/wave-0/01-live-bugs.md
// rule 13). Pure, so `bun test` can drive it; the service worker feeds it what is showing.

export type PushKind = 'needs' | 'updates'

/** A notification this worker is showing, as its `data` records it. */
export interface Shown {
  tag: string
  kind: PushKind
  /** When it was shown, ms. */
  at: number
  /** How many items a fold stands for. */
  count?: number
}

/** What the server pushed: words, a page and a tag (never an answer). */
export interface Incoming {
  title: string
  body: string
  url: string
  tag: string
  kind: PushKind
}

export interface ToShow extends Incoming {
  count?: number
}

export interface Plan {
  close: string[]
  show: ToShow[]
}

export const NEEDS_FOLD_TAG = 'hq-needs-more'
export const UPDATES_FOLD_TAG = 'hq-updates-fold'
/** Needs you notifications that show on their own before the rest fold. */
export const NEEDS_SHOWN_MAX = 5
/** Updates within the window that show on their own; the next one folds them all. */
export const UPDATES_SHOWN_MAX = 3
export const UPDATES_WINDOW_MS = 60 * 60 * 1000
const INBOX_URL = '/notifications'

const needsFold = (n: number): ToShow => ({
  title: `${n} more waiting for you`,
  body: 'Open HQ to see them.',
  url: INBOX_URL,
  tag: NEEDS_FOLD_TAG,
  kind: 'needs',
  count: n,
})

const updatesFold = (n: number): ToShow => ({
  title: `${n} updates from HQ`,
  body: 'Tap to read.',
  url: INBOX_URL,
  tag: UPDATES_FOLD_TAG,
  kind: 'updates',
  count: n,
})

/** What to close and what to show for one incoming push. */
export function planPush(shown: Shown[], push: Incoming, now: number): Plan {
  // A replay of an item already showing replaces it in place.
  if (shown.some((s) => s.tag === push.tag)) return { close: [], show: [push] }
  return push.kind === 'needs' ? planNeeds(shown, push) : planUpdates(shown, push, now)
}

function planNeeds(shown: Shown[], push: Incoming): Plan {
  const own = shown.filter((s) => s.kind === 'needs' && s.tag !== NEEDS_FOLD_TAG).sort((a, b) => a.at - b.at)
  if (own.length < NEEDS_SHOWN_MAX) return { close: [], show: [push] }
  const folded = shown.find((s) => s.tag === NEEDS_FOLD_TAG)?.count ?? 0
  // The newest five stay on their own: the oldest joins the fold.
  return { close: [own[0].tag], show: [push, needsFold(folded + 1)] }
}

function planUpdates(shown: Shown[], push: Incoming, now: number): Plan {
  const recent = shown.filter((s) => s.kind === 'updates' && now - s.at < UPDATES_WINDOW_MS)
  const fold = recent.find((s) => s.tag === UPDATES_FOLD_TAG)
  const own = recent.filter((s) => s.tag !== UPDATES_FOLD_TAG)
  if (!fold && own.length < UPDATES_SHOWN_MAX) return { close: [], show: [push] }
  const total = (fold?.count ?? 0) + own.length + 1
  return { close: own.map((s) => s.tag), show: [updatesFold(total)] }
}
