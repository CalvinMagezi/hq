import { parseSqliteUtc } from '~/lib/tasksApi'
import { relTime } from '~/lib/time'
import { t, type StringKey } from '~/strings'
import type {
  ActivityGap,
  ActivityNote,
  InitiativeActivity,
  NoteAttribution,
  ReviewState,
  TaskActivity,
} from '~/lib/taskActivityApi'

/** Task times are SQLite UTC ("2026-10-15 14:39:16"); note records are RFC 3339 ("...T09:01:08+00:00"). */
export function parseStamp(text: string): Date {
  return /[zZ]$|[+-]\d{2}:\d{2}$/.test(text) ? new Date(text) : parseSqliteUtc(text)
}

/** The whole span phrase ("12 min ago", "3 h ago"); "Not known" for nothing or something unreadable. */
export function ageLabel(text: string | null | undefined, nowMs: number = Date.now()): string {
  if (!text) return t('time_unknown')
  const then = parseStamp(text).getTime()
  if (Number.isNaN(then)) return t('time_unknown')
  return relTime(then, nowMs)
}

export interface Badge {
  label: string
  /** a-* and leaf.css classes. Accepted is solid, unreviewed is hollow and dashed, so the two never look alike. */
  className: string
  /** What the badge means, for the title attribute. */
  title: string
}

const BASE = 'a-kind sm'

const REVIEW_BADGES: Record<ReviewState, Badge> = {
  accepted: {
    label: 'Accepted',
    className: 'a-kind',
    title: 'Someone accepted this version',
  },
  reviewed: {
    label: 'Reviewed',
    className: BASE,
    title: 'Someone looked at this version without accepting it',
  },
  disputed: {
    label: 'Disputed',
    className: 'a-state needs',
    title: 'Someone disputed this version',
  },
  superseded: {
    label: 'Superseded',
    className: `${BASE} l-line`,
    title: 'A later note replaces this one',
  },
  unreviewed: {
    label: 'Unreviewed',
    className: `${BASE} l-dashed`,
    title: 'No one has reviewed this version',
  },
}

const NO_RECORD_BADGE: Badge = {
  label: 'No record',
  className: `${BASE} l-dotted`,
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

const GAP_LINES: Record<string, StringKey> = {
  note_unattributed: 'act_gap_note_unattributed',
  note_changed_outside_tools: 'act_gap_note_changed_outside_tools',
  note_missing: 'act_gap_note_missing',
  note_without_session: 'act_gap_note_without_session',
  note_from_another_task: 'act_gap_note_from_another_task',
  session_not_registered: 'act_gap_session_not_registered',
  in_progress_without_a_holder: 'act_gap_in_progress_without_a_holder',
  notes_not_listed: 'act_gap_notes_not_listed',
}

/** Gaps about the task itself, whose sentence names nothing else. */
const NO_SUBJECT_GAPS: readonly string[] = ['in_progress_without_a_holder', 'notes_not_listed']

/** One plain sentence for a gap, in the owner's words; the server's own detail is for tools, not people. */
export function gapLine(gap: Pick<ActivityGap, 'kind' | 'subject' | 'detail'>): string {
  const key = GAP_LINES[gap.kind]
  const needsSubject = !NO_SUBJECT_GAPS.includes(gap.kind)
  if (!key || (needsSubject && !gap.subject)) return gap.detail
  const subject = gap.subject ?? ''
  return t(key, { note: subject, subject })
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

/** The closing line: when anything last happened, and that a note edit is not progress. */
export function lastActivityLine(freshness: Record<string, string | null>, nowMs: number = Date.now()): string {
  const newest = Object.values(freshness)
    .filter((at): at is string => !!at)
    .map((at) => ({ at, ms: parseStamp(at).getTime() }))
    .filter((x) => !Number.isNaN(x.ms))
    .sort((x, y) => y.ms - x.ms)[0]
  return newest ? t('act_last', { ago: ageLabel(newest.at, nowMs) }) : t('act_quiet')
}

/** Whether an initiative has anything the briefing can say beyond its task list. */
export function briefingIsEmpty(view: Pick<InitiativeActivity, 'tasks_shown'>): boolean {
  return view.tasks_shown === 0
}
