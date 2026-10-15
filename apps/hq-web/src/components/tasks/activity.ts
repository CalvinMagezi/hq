import { parseSqliteUtc } from '~/lib/tasksApi'
import type {
  ActivityGap,
  ActivityNote,
  InitiativeActivity,
  NoteAttribution,
  ReviewState,
  TaskActivity,
} from '~/lib/taskActivityApi'

const MS_PER_MINUTE = 60_000
const MINUTES_PER_HOUR = 60
const HOURS_PER_DAY = 24

/** Task times are SQLite UTC ("2026-10-15 14:39:16"); note records are RFC 3339 ("...T09:01:08+00:00"). */
export function parseStamp(text: string): Date {
  return /[zZ]$|[+-]\d{2}:\d{2}$/.test(text) ? new Date(text) : parseSqliteUtc(text)
}

/** "just now", "12m ago", "3h ago", "2d ago"; "unknown" for nothing or something unreadable. */
export function ageLabel(text: string | null | undefined, nowMs: number = Date.now()): string {
  if (!text) return 'unknown'
  const then = parseStamp(text).getTime()
  if (Number.isNaN(then)) return 'unknown'
  const minutes = Math.max(0, Math.floor((nowMs - then) / MS_PER_MINUTE))
  if (minutes < 1) return 'just now'
  if (minutes < MINUTES_PER_HOUR) return `${minutes}m ago`
  const hours = Math.floor(minutes / MINUTES_PER_HOUR)
  if (hours < HOURS_PER_DAY) return `${hours}h ago`
  return `${Math.floor(hours / HOURS_PER_DAY)}d ago`
}

export interface Badge {
  label: string
  /** Tailwind classes. Accepted is solid, unreviewed is hollow and dashed, so the two never look alike. */
  className: string
  /** What the badge means, for the title attribute. */
  title: string
}

const BASE = 'px-1.5 py-0.5 rounded-md text-[10px] font-semibold uppercase tracking-wide border'

const REVIEW_BADGES: Record<ReviewState, Badge> = {
  accepted: {
    label: 'Accepted',
    className: `${BASE} bg-emerald-500/15 text-emerald-300 border-emerald-500/40`,
    title: 'Someone accepted this version',
  },
  reviewed: {
    label: 'Reviewed',
    className: `${BASE} bg-sky-500/10 text-sky-300 border-sky-500/30`,
    title: 'Someone looked at this version without accepting it',
  },
  disputed: {
    label: 'Disputed',
    className: `${BASE} bg-rose-500/15 text-rose-300 border-rose-500/40`,
    title: 'Someone disputed this version',
  },
  superseded: {
    label: 'Superseded',
    className: `${BASE} bg-white/5 text-neutral-500 border-white/10 line-through`,
    title: 'A later note replaces this one',
  },
  unreviewed: {
    label: 'Unreviewed',
    className: `${BASE} bg-transparent text-neutral-400 border-dashed border-neutral-600`,
    title: 'No one has reviewed this version',
  },
}

const NO_RECORD_BADGE: Badge = {
  label: 'No record',
  className: `${BASE} bg-transparent text-neutral-500 border-dotted border-neutral-700`,
  title: 'This version of the file has no provenance record, so there is nothing to review',
}

/** The review badge for a note. A version with no record is "No record", never "Unreviewed". */
export function reviewBadge(note: Pick<ActivityNote, 'review_state'>): Badge {
  return note.review_state ? REVIEW_BADGES[note.review_state] : NO_RECORD_BADGE
}

/** Who wrote this version, in words. Anything not recorded stays "unknown": nobody is guessed. */
export function authorLabel(note: Pick<ActivityNote, 'attribution' | 'actor' | 'source_kind'>): string {
  switch (note.attribution) {
    case 'recorded':
      return note.source_kind && note.source_kind !== 'unknown' ? `${note.actor} (${note.source_kind})` : (note.actor ?? 'unknown')
    case 'unattributed':
    case 'no_record':
      return 'unknown'
    case 'changed_outside_tools':
      return 'unknown (edited after its last recorded write)'
    case 'missing':
      return 'file is gone'
  }
}

export const ATTRIBUTION_GAP_ATTRIBUTIONS: readonly NoteAttribution[] = ['unattributed', 'no_record', 'changed_outside_tools']

const GAP_TITLES: Record<string, string> = {
  note_unattributed: 'Author unknown',
  note_changed_outside_tools: 'Edited outside HQ',
  note_missing: 'Linked note is gone',
  note_without_session: 'No session recorded',
  note_from_another_task: 'Written under another task',
  session_not_registered: 'Session not in the registry',
  in_progress_without_a_holder: 'Nobody holds it',
  notes_not_listed: 'More notes than shown',
}

/** A short title for a gap kind; an unknown kind is shown as it came. */
export function gapTitle(gap: Pick<ActivityGap, 'kind'>): string {
  return GAP_TITLES[gap.kind] ?? gap.kind.replaceAll('_', ' ')
}

/**
 * What the panel has to show for a task:
 * - `contributors`: someone is on record as having worked on it
 * - `unattributed_only`: notes are linked or changed but none has a known author
 * - `none`: nothing recorded at all
 */
export type PanelKind = 'contributors' | 'unattributed_only' | 'none'

export function panelKind(view: Pick<TaskActivity, 'contributors' | 'notes' | 'holders' | 'sessions'>): PanelKind {
  if (view.contributors.length > 0) return 'contributors'
  if (view.notes.length > 0 || view.holders.length > 0 || view.sessions.length > 0) return 'unattributed_only'
  return 'none'
}

/** Notes whose author is not known, for the count line. */
export function unattributedCount(notes: readonly ActivityNote[]): number {
  return notes.filter((n) => ATTRIBUTION_GAP_ATTRIBUTIONS.includes(n.attribution)).length
}

const REVIEW_ORDER: readonly ReviewState[] = ['accepted', 'reviewed', 'unreviewed', 'disputed', 'superseded']

/** "2 accepted, 1 unreviewed": states in a fixed order, zero counts left out. */
export function reviewSummary(counts: Partial<Record<ReviewState, number>>): string {
  const parts = REVIEW_ORDER.filter((s) => (counts[s] ?? 0) > 0).map((s) => `${counts[s]} ${s}`)
  return parts.length > 0 ? parts.join(', ') : 'no reviews recorded'
}

/** The freshest source times, labelled, for the "as of" line. Sources with nothing are named, not dated. */
export function freshnessLines(freshness: Record<string, string | null>, nowMs: number = Date.now()): string[] {
  return Object.entries(freshness).map(([source, at]) => `${source}: ${at ? ageLabel(at, nowMs) : 'nothing yet'}`)
}

/** Whether an initiative has anything the briefing can say beyond its task list. */
export function briefingIsEmpty(view: Pick<InitiativeActivity, 'tasks_shown'>): boolean {
  return view.tasks_shown === 0
}
