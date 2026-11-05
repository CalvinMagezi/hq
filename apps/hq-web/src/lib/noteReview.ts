// Who wrote a note and whether the owner checked it, from GET /api/note/versions and
// POST /api/note/review (spec 03 "Who wrote it"). Plain words only: the tag names the state, never the
// record behind it.

import { HqHttpError, hqFetch, hqJson } from './hqAuth'

export interface ReviewEvent {
  state: string
  reviewer: string
  reason?: string | null
  at: string
}

export interface NoteVersion {
  id: number
  content_hash: string
  is_current: boolean
  actor?: string | null
  source_kind: string
  written_at: string
  unattributed?: boolean
  review_state: string
  reviewed_by?: string | null
  reviewed_at?: string | null
  review_reason?: string | null
  review_history: ReviewEvent[]
}

export interface NoteVersions {
  exists: boolean
  current_version_recorded: boolean
  versions: NoteVersion[]
}

export type ReviewTagKind = 'unreviewed' | 'accepted' | 'reviewed' | 'disputed' | 'superseded' | 'unknown'

export type WhoKind = { kind: 'you' | 'hq' | 'imported' | 'unknown' } | { kind: 'helper'; name: string }

export function fetchNoteVersions(path: string): Promise<NoteVersions> {
  return hqJson(`/api/note/versions?path=${encodeURIComponent(path)}`)
}

/** Looks right is `accepted`; Something's wrong is `disputed` and needs a reason. */
export async function reviewNote(path: string, verdict: 'accepted' | 'disputed', reason?: string): Promise<void> {
  if (verdict === 'disputed' && !reason?.trim()) throw new Error('a dispute needs a reason')
  const res = await hqFetch('/api/note/review', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path, verdict, reason: reason?.trim() || undefined }),
  })
  if (!res.ok) {
    const b = await res.json().catch(() => ({}))
    throw new HqHttpError(b.error || `${res.status} ${res.statusText}`, res.status)
  }
}

/** The version on disk now, or null when the note was changed outside HQ or never recorded. */
export function currentVersion(v: NoteVersions): NoteVersion | null {
  if (!v.current_version_recorded) return null
  return v.versions.find((x) => x.is_current) ?? null
}

export function reviewTag(version: NoteVersion | null): ReviewTagKind {
  if (!version || version.unattributed) return 'unknown'
  switch (version.review_state) {
    case 'accepted':
      return 'accepted'
    case 'reviewed':
      return 'reviewed'
    case 'disputed':
      return 'disputed'
    case 'superseded':
      return 'superseded'
    default:
      return 'unreviewed'
  }
}

export function whoWrote(version: NoteVersion | null): WhoKind {
  if (!version || version.unattributed) return { kind: 'unknown' }
  switch (version.source_kind) {
    case 'human':
      return { kind: 'you' }
    case 'agent':
      return version.actor ? { kind: 'helper', name: version.actor } : { kind: 'unknown' }
    case 'generated':
    case 'system':
      return { kind: 'hq' }
    case 'imported':
      return { kind: 'imported' }
    default:
      return { kind: 'unknown' }
  }
}

/** Only the owner's own typing can read as checked; a paste or an import never does (W19). */
export function offersLooksRight(version: NoteVersion | null): boolean {
  return !!version && !version.unattributed && version.review_state !== 'accepted'
}

/** Newest first, which is the order the server answers in. */
export function historyOrder(v: NoteVersions): NoteVersion[] {
  return [...v.versions].sort((a, b) => Date.parse(b.written_at) - Date.parse(a.written_at))
}
