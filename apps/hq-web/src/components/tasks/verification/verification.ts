// What the verification screens say, kept apart from how they look so it can be tested. The
// meanings are in docs/TASK_VERIFICATION.md; the wording here is for someone checking a phone
// between other things, so it says what happened and what is theirs to do.
import type {
  ApplyPreview,
  Calibration,
  Criterion,
  QueueHealth,
  Rollout,
  SweepBucket,
  SweepEstimate,
  SweepAction,
  SweepProgress,
  SweepScope,
  SweepStatus,
  TierCalibration,
  WaitingFor,
} from '~/lib/verificationApi'

/** Owner first: it is the only one that is waiting on the person reading. */
export const WAITING_ORDER: readonly WaitingFor[] = ['owner', 'verifier', 'ci', 'budget', 'worker']

export const WAITING_LABEL: Record<WaitingFor, string> = {
  owner: 'Needs you',
  verifier: 'Queued',
  ci: 'Checks running',
  budget: 'Waiting for budget',
  worker: 'With the worker',
}

export interface WaitingChip {
  who: WaitingFor
  label: string
  count: number
  /** Counts against the person reading. */
  yours: boolean
}

/** One chip per party that has something waiting, the owner's first. */
export function waitingChips(backlog: QueueHealth['backlog']): WaitingChip[] {
  return WAITING_ORDER.flatMap((who) => {
    const count = backlog[who] ?? 0
    return count > 0 ? [{ who, label: WAITING_LABEL[who], count, yours: who === 'owner' }] : []
  })
}

export const BUCKET_ORDER: readonly SweepBucket[] = [
  'passed',
  'needs_review',
  'failed',
  'no_artifact',
  'undecidable',
  'needs_you',
]

export const BUCKET_INFO: Record<SweepBucket, { label: string; does: string }> = {
  unplanned: { label: 'Not looked at yet', does: '' },
  needs_review: { label: 'Need a reviewer', does: 'Judged when you run it, within your budget' },
  passed: { label: 'Passed', does: 'Completed, with the evidence level recorded' },
  failed: { label: 'Failed', does: 'Back to to_do with what is wrong' },
  no_artifact: { label: 'Nothing to check', does: 'Back to to_do with what to link; never completed' },
  undecidable: { label: 'Could not decide', does: 'Left where they are' },
  needs_you: { label: 'Need you', does: 'Stay in your list' },
}

export interface BucketRow {
  bucket: SweepBucket
  label: string
  does: string
  count: number
}

/** The buckets that have anything in them, in the order a person reads them. */
export function bucketRows(buckets: Partial<Record<SweepBucket, number>> | undefined): BucketRow[] {
  return BUCKET_ORDER.flatMap((bucket) => {
    const count = buckets?.[bucket] ?? 0
    return count > 0 ? [{ bucket, count, ...BUCKET_INFO[bucket] }] : []
  })
}

export interface SweepStageInfo {
  label: string
  /** What is going on, or what to do next. */
  hint: string
  /** The controls that make sense now, in the order they are shown. */
  actions: SweepAction[]
}

/** Where a sweep is and what can be done with it. Anything else is refused by the server. */
export function sweepStage(status: SweepStatus, preview: ApplyPreview | null): SweepStageInfo {
  switch (status) {
    case 'planning':
      return { label: 'Reading the evidence', hint: 'No model is used and nothing moves while HQ plans.', actions: ['cancel'] }
    case 'planned':
      return { label: 'Plan ready', hint: 'Read the groups, then run it. Running records verdicts and still moves nothing.', actions: ['run', 'cancel'] }
    case 'running':
      return { label: 'Verifying', hint: 'Verdicts are recorded as they come. Nothing moves until you apply.', actions: ['pause', 'cancel'] }
    case 'paused':
      return { label: 'Paused', hint: 'It stopped between tasks. Run it again to continue.', actions: ['run', 'cancel'] }
    case 'ready':
      return {
        label: 'Ready to apply',
        hint: preview
          ? `Applying moves ${preview.will_move} tasks and leaves ${preview.will_keep} where they are.`
          : 'Applying moves the tasks in the groups above.',
        actions: ['apply', 'cancel'],
      }
    case 'applying':
      return { label: 'Applying', hint: 'Each task is checked again first, because the work may have changed.', actions: [] }
    case 'applied':
      return { label: 'Applied', hint: 'Undo puts back what has not changed since.', actions: ['undo'] }
    case 'undone':
      return { label: 'Undone', hint: 'Restored tasks are not verified again until they are submitted again.', actions: [] }
    case 'cancelled':
      return { label: 'Cancelled', hint: 'Nothing was moved.', actions: [] }
  }
}

export interface ProgressInfo {
  done: number
  total: number
  percent: number
  label: string
}

/** How far the current phase has got, or null when nothing is running. */
export function sweepProgress(status: SweepStatus, p: SweepProgress | undefined): ProgressInfo | null {
  if (!p || p.total === 0) return null
  const s = p.states
  const n = (k: keyof typeof s) => s[k] ?? 0
  let done: number
  let label: string
  if (status === 'planning') {
    done = p.total - n('unplanned')
    label = 'planned'
  } else if (status === 'running' || status === 'paused') {
    done = p.total - n('unplanned') - n('planned')
    label = 'verified'
  } else if (status === 'applying') {
    done = n('applied') + n('kept') + n('stale')
    label = 'applied'
  } else {
    return null
  }
  const percent = Math.min(100, Math.round((done / p.total) * 100))
  return { done, total: p.total, percent, label: `${done} of ${p.total} ${label}` }
}

/** The cost line for a plan: what the reviewer runs would take, and where the number comes from. */
export function estimateLine(e: SweepEstimate | undefined): string | null {
  if (!e || e.reviewer_runs === 0) return null
  const runs = `${e.reviewer_runs} reviewer ${e.reviewer_runs === 1 ? 'run' : 'runs'}`
  const cost = `about $${e.usd.toFixed(2)} and ${e.minutes} ${e.minutes === 1 ? 'minute' : 'minutes'}`
  const budget = e.reviewer_enabled
    ? e.monthly_ceiling_usd === null
      ? ''
      : `, against $${e.spent_this_month_usd.toFixed(2)} of $${e.monthly_ceiling_usd.toFixed(2)} spent this month`
    : '. No reviewer budget is set, so these will wait'
  return `${runs}, ${cost}${budget}. This is an estimate: ${e.basis}.`
}

export function scopeLabel(scope: SweepScope, initiativeName?: string): string {
  switch (scope.kind) {
    case 'all':
      return 'Everything waiting for review'
    case 'initiative':
      return initiativeName ? `Everything waiting in ${initiativeName}` : 'Everything waiting in one initiative'
    case 'since':
      return `Waiting since ${scope.date}`
    case 'tasks':
      return `${scope.ids.length} picked ${scope.ids.length === 1 ? 'task' : 'tasks'}`
  }
}

export type ScopeChoice = 'all' | 'initiative' | 'since' | 'selected'

export interface ScopeInputs {
  initiativeId: string | null
  /** Tasks ticked on the board. */
  selectedIds: readonly string[]
  /** YYYY-MM-DD as typed. */
  since: string
}

const DATE = /^\d{4}-\d{2}-\d{2}$/

/** The scope for a choice, or null while the choice is missing what it needs. */
export function pickScope(choice: ScopeChoice, inputs: ScopeInputs): SweepScope | null {
  switch (choice) {
    case 'all':
      return { kind: 'all' }
    case 'initiative':
      return inputs.initiativeId ? { kind: 'initiative', id: inputs.initiativeId } : null
    case 'since':
      return DATE.test(inputs.since.trim()) ? { kind: 'since', date: inputs.since.trim() } : null
    case 'selected':
      return inputs.selectedIds.length > 0 ? { kind: 'tasks', ids: [...inputs.selectedIds] } : null
  }
}

const PERCENT = (rate: number) => `${Math.round(rate * 100)}%`

/** What a tier's calibration says, in a line. */
export function calibrationLine(t: TierCalibration): string {
  const head = `Tier ${t.tier}: ${t.state === 'earned' ? 'earned' : t.state === 'withdrawn' ? 'withdrawn' : 'not enough data yet'}`
  const bits = [`${t.audits} audits`, `${t.disagreements} disagreed`]
  if (t.disagreement_upper !== null) bits.push(`upper bound ${PERCENT(t.disagreement_upper)}`)
  return `${head}. ${bits.join(', ')}.`
}

/** The running error estimate for the audit view. It never states a rate it has no sample for. */
const AUDITS_FOR_A_RATE = 30

export function auditEstimate(cal: Calibration): string {
  const audits = cal.tiers.reduce((n, t) => n + t.audits, 0)
  const disagreed = cal.tiers.reduce((n, t) => n + t.disagreements, 0)
  if (audits === 0) return 'No audits answered yet, so there is no error estimate.'
  const sample = `${audits} ${audits === 1 ? 'audit' : 'audits'}`
  // A rate is only stated for a tier with a sample of its own: a thin tier's bound is wide and
  // would make a well-audited one look worse than it is.
  const sampled = cal.tiers.filter((t) => t.audits >= AUDITS_FOR_A_RATE && t.disagreement_upper !== null)
  if (sampled.length === 0) {
    return `${disagreed} of ${sample} disagreed. Too few to say how often HQ is wrong: it takes about ${AUDITS_FOR_A_RATE} in one tier.`
  }
  const upper = sampled.reduce((m, t) => Math.max(m, t.disagreement_upper ?? 0), 0)
  const names = sampled.map((t) => `tier ${t.tier}`).join(' and ')
  return `${disagreed} of ${sample} disagreed. For ${names}, HQ is wrong at most ${PERCENT(upper)} of the time, with 95% confidence.`
}

/** "5h 20m", "3d 4h" or "now" for the time until a unix-second moment. */
export function countdown(releaseAt: number, nowMs: number): string {
  const secs = Math.max(0, Math.floor(releaseAt - nowMs / 1000))
  if (secs < 60) return 'now'
  const mins = Math.floor(secs / 60)
  if (mins < 60) return `${mins}m`
  const hours = Math.floor(mins / 60)
  if (hours < 48) return `${hours}h ${mins % 60}m`
  return `${Math.floor(hours / 24)}d ${hours % 24}h`
}

export function ageLabel(hours: number | null): string {
  if (hours === null) return 'unknown'
  if (hours < 1) return 'under an hour'
  if (hours < 48) return `${hours}h`
  return `${Math.floor(hours / 24)} days`
}

const LEVELS: Record<string, string> = {
  machine_peer: 'machine checks and an independent review',
  machine: 'machine checks',
  peer: 'an independent review',
  owner: 'you',
  none: 'no evidence',
}

export const levelLabel = (level: string): string => LEVELS[level] ?? level

/** "3 of 5 criteria met". */
export function criteriaSummary(criteria: readonly Criterion[]): string {
  if (criteria.length === 0) return 'No acceptance criteria were stated'
  const met = criteria.filter((c) => c.result === 'pass').length
  return `${met} of ${criteria.length} ${criteria.length === 1 ? 'criterion' : 'criteria'} met`
}

/** What the queue health says in plain sentences, in the order they matter. */
export function queueLines(h: QueueHealth): string[] {
  const out: string[] = [...h.alerts]
  out.push(`${h.completed_24h} completed by HQ and ${h.verdicts_24h} verdicts in the last 24 hours.`)
  if (h.est_clear_hours !== null) out.push(`The queue behind the verifier clears in about ${h.est_clear_hours} hours at this rate.`)
  if (h.oldest_owner_hours !== null) out.push(`The oldest item waiting on you is ${ageLabel(h.oldest_owner_hours)} old.`)
  out.push(
    h.monthly_ceiling_usd === null
      ? 'No reviewer budget is set, so no model is called.'
      : `Reviewer spend this month: $${h.spent_this_month_usd.toFixed(2)} of $${h.monthly_ceiling_usd.toFixed(2)}.`
  )
  return out
}

/** Whether the screens have anything to say: with verification off they stay out of the way. */
export const verificationActive = (mode: string | undefined): boolean => mode === 'shadow' || mode === 'enforce'

export interface RolloutView {
  /** Where the instance is, as a sentence. */
  stage: string
  warnings: string[]
  /** The next step, or null when every tier is enforced. */
  next: { label: string; change: string; verdict: string } | null
  /** Unmet conditions first: they are what the owner can act on. */
  gates: Rollout['gates']
}

/** What the "turning it on" section says: where this is, whether the next step is open, what is missing. */
export function rolloutView(r: Rollout): RolloutView {
  const open = r.gates.filter((g) => !g.met).length
  const verdict =
    r.next === null
      ? ''
      : r.ready
        ? 'Every condition is met. This is the one setting to change.'
        : open === 1
          ? 'One condition is not met yet.'
          : `${open} conditions are not met yet.`
  return {
    stage: r.stage_label,
    warnings: r.warnings,
    next: r.next ? { ...r.next, verdict } : null,
    gates: [...r.gates.filter((g) => !g.met), ...r.gates.filter((g) => g.met)],
  }
}
