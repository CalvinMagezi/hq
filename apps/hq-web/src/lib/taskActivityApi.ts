import { hqFetch, readJson } from '~/lib/hqAuth'

/** How a note's author is known. Anything but `recorded` means the author is not known for this version. */
export type NoteAttribution = 'recorded' | 'unattributed' | 'changed_outside_tools' | 'no_record' | 'missing'

export type ReviewState = 'unreviewed' | 'reviewed' | 'accepted' | 'disputed' | 'superseded'

/** One note tied to a task, as `GET /api/tasks/{id}/activity` reports it. */
export interface ActivityNote {
  path: string
  attribution: NoteAttribution
  /** `produced`, `origin` or `related` when the task links the note. */
  link: string | null
  /** The `type:` the note declares for itself, if any. */
  type: string | null
  /** The fields below exist only when the version on disk has a provenance record. */
  version_id?: number
  written_at?: string
  actor?: string
  source_kind?: string
  session?: string | null
  task?: string | null
  recorded_late?: boolean
  review_state?: ReviewState
  reviewed_by?: string | null
  reviewed_at?: string | null
  review_reason?: string | null
  /** Initiative view only. */
  task_display_id?: string
}

export interface ActivityContributor {
  actor: string
  source_kinds: string[]
  notes: number
  leases: number
  last_active: string | null
}

export interface ActivityGap {
  kind: string
  detail: string
  subject: string | null
}

export interface ActivityLease {
  lease_id: string
  actor: string
  harness: string
  branch: string
  harness_session: string | null
  started_at: string
  last_heartbeat_at: string
  ended_at: string | null
  end_reason: string | null
  live: boolean
}

export interface ActivitySession {
  id: string
  /** False when the session registry has no row for a linked id. */
  registered?: boolean
  harness?: string
  agent?: string
  purpose?: string
  status?: string
  running?: boolean
  last_seen_at?: string | null
  goal?: string | null
  via: string[]
}

export interface TaskActivity {
  scope: 'task'
  generated_at: string
  basis: string
  task: {
    id: string
    display_id: string
    title: string
    status: string
    priority: string | null
    assignees: string[]
    blocked_by: string[]
    updated_at: string
  }
  awaiting_review: boolean
  blocker: { reason: string | null; waiting_on: string | null; since: string | null } | null
  checkpoint: {
    summary: string
    next_step: string
    open_questions: string
    files: string[]
    actor: string
    at: string
  } | null
  holders: ActivityLease[]
  recent_leases: ActivityLease[]
  sessions: ActivitySession[]
  notes: ActivityNote[]
  notes_total: number
  review_counts: Partial<Record<ReviewState, number>>
  decisions_and_findings: Partial<Record<ReviewState, ActivityNote[]>>
  open_questions: { text: string; recorded_by: string; at: string; source: string }[]
  contributors: ActivityContributor[]
  gaps: ActivityGap[]
  gaps_total: number
  freshness: Record<string, string | null>
  sources_with_nothing: string[]
}

export interface InitiativeActivityRow {
  id: string
  display_id: string
  title: string
  status: string
  priority: string | null
  assignees: string[]
  blocked_by: string[]
  holders: string[]
  last_checkpoint_at: string | null
  notes: number
  review_counts: Partial<Record<ReviewState, number>>
  gaps: number
}

export interface InitiativeActivity {
  scope: 'initiative'
  generated_at: string
  basis: string
  initiative: { id: string; name: string; prefix: string; space: { slug: string; name: string } | null }
  tasks_shown: number
  status_counts: Record<string, number>
  tasks: InitiativeActivityRow[]
  awaiting_review: { id: string; display_id: string; title: string }[]
  blocked: { id: string; display_id: string; title: string; reason: string | null; waiting_on: string | null }[]
  review_counts: Partial<Record<ReviewState, number>>
  recent_notes: ActivityNote[]
  contributors: ActivityContributor[]
  gaps_total: number
  gap_kinds: Record<string, number>
  freshness: Record<string, string | null>
}

export async function fetchTaskActivityClient(taskId: string): Promise<TaskActivity> {
  return readJson(await hqFetch(`/api/tasks/${encodeURIComponent(taskId)}/activity`))
}

export async function fetchInitiativeActivityClient(initiativeId: string): Promise<InitiativeActivity> {
  return readJson(await hqFetch(`/api/initiatives/${encodeURIComponent(initiativeId)}/activity`))
}
