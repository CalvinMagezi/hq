// Task verification, the owner's side: HQ checks finished work against evidence the worker could
// not write, and asks the owner only about exceptions. The server is `/api/verification/*`
// (crates/hq-web/src/verification_api.rs); docs/TASK_VERIFICATION.md says what each thing means.
import { hqJson } from './hqAuth'

export type VerificationMode = 'off' | 'shadow' | 'enforce'

/** Who a waiting task is waiting on. Only `owner` counts against the owner. */
export type WaitingFor = 'worker' | 'ci' | 'verifier' | 'budget' | 'owner'

export interface CanaryStats {
  runs: number
  defects: number
  caught: number
  controls: number
  passed: number
  catch_rate: number | null
  false_reject_rate: number | null
}

export type Autonomy = 'earned' | 'insufficient' | 'withdrawn'

export interface TierCalibration {
  tier: number
  audits: number
  disagreements: number
  disagreement_upper: number | null
  escapes: number
  state: Autonomy
  why: string[]
}

export interface Calibration {
  verifier_config: string
  canaries: CanaryStats
  tiers: TierCalibration[]
  open_audits: number
}

export interface Deferral {
  waiting_for: WaitingFor
  reason: string
  tasks: number
}

export interface QueueHealth {
  mode: VerificationMode
  backlog: Partial<Record<WaitingFor, number>>
  owner_by_kind: Record<string, number>
  oldest_owner_hours: number | null
  verdicts_24h: number
  completed_24h: number
  est_clear_hours: number | null
  deferrals: Deferral[]
  spent_this_month_usd: number
  monthly_ceiling_usd: number | null
  alerts: string[]
}

export type RolloutStage = 'off' | 'shadow' | 'tier_0' | 'tier_1' | 'all_tiers'

/** One condition for the next step of turning verification on. */
export interface RolloutGate {
  label: string
  met: boolean
  /** The numbers behind it, in a sentence. */
  detail: string
}

/** Shadow verdicts of one tier next to what someone else then did with the work. */
export interface ShadowTier {
  tier: number
  compared: number
  agreed: number
  false_passes: number
  false_rejects: number
  false_pass_upper: number | null
}

export interface Rollout {
  stage: RolloutStage
  stage_label: string
  /** The next step and the setting that takes it; null once every tier is enforced. */
  next: { label: string; change: string } | null
  gates: RolloutGate[]
  ready: boolean
  warnings: string[]
  shadow: ShadowTier[]
}

export interface VerificationStatus {
  mode: VerificationMode
  autonomy_gate: 'off' | 'demote' | 'require'
  confirm_over: number
  calibration: Calibration
  queue: QueueHealth
  rollout: Rollout
  holds: number
  active_sweeps: number
}

export interface WaitingItem {
  task_id: string
  display_id: string
  title: string
  initiative_id: string
  tier: number
  who: WaitingFor
  kind: string
  reason: string
  since: string
}

export interface CompletedGroup {
  initiative_id: string
  initiative: string
  count: number
  by_level: Record<string, number>
  tier2: number
}

export interface ReworkedTask {
  task_id: string
  display_id: string
  title: string
  returns: number
  last_finding: string
  status: string
}

export interface VetoWindow {
  task_id: string
  display_id: string
  title: string
  /** Unix seconds. */
  release_at: number
}

export interface BulkOffer {
  count: number
  text: string
  command: string
}

export interface Briefing {
  since: string
  mode: VerificationMode
  needs_you: WaitingItem[]
  needs_you_total: number
  bulk_offer: BulkOffer | null
  completed: CompletedGroup[]
  completed_total: number
  reworked: ReworkedTask[]
  reworked_total: number
  veto_windows: VetoWindow[]
  audit_pool: number
  open_audits: number
  health: QueueHealth
}

export type SweepScope =
  | { kind: 'all' }
  | { kind: 'initiative'; id: string }
  | { kind: 'since'; date: string }
  | { kind: 'tasks'; ids: string[] }

export type SweepStatus =
  | 'planning'
  | 'planned'
  | 'running'
  | 'paused'
  | 'cancelled'
  | 'ready'
  | 'applying'
  | 'applied'
  | 'undone'

export type SweepBucket =
  | 'unplanned'
  | 'needs_review'
  | 'passed'
  | 'failed'
  | 'no_artifact'
  | 'undecidable'
  | 'needs_you'

export type SweepItemState = 'unplanned' | 'planned' | 'verdict' | 'applied' | 'kept' | 'stale' | 'undone' | 'left_alone'

export interface SweepEstimate {
  reviewer_runs: number
  usd: number
  minutes: number
  basis: string
  reviewer_enabled: boolean
  monthly_ceiling_usd: number | null
  spent_this_month_usd: number
}

export interface SweepPlan {
  total?: number
  buckets?: Partial<Record<SweepBucket, number>>
  tier2?: number
  would_wait_on_you?: number
  estimate?: SweepEstimate
}

export interface Sweep {
  id: string
  scope: SweepScope
  status: SweepStatus
  plan: SweepPlan
  note: string
  created_at: string
  updated_at: string
  applied_at: string | null
  undone_at: string | null
}

export interface SweepItem {
  sweep_id: string
  task_id: string
  display_id: string
  title: string
  tier: number
  bucket: SweepBucket
  state: SweepItemState
  finding: string
  detail: string
  status_before: string
  applied_status: string | null
}

export interface SweepProgress {
  total: number
  buckets: Partial<Record<SweepBucket, number>>
  states: Partial<Record<SweepItemState, number>>
}

export interface ApplyPreview {
  will_move: number
  will_keep: number
  confirm_over: number
  needs_confirmation: boolean
}

export interface SweepDetail {
  sweep: Sweep
  progress: SweepProgress
  items: SweepItem[]
  apply_preview: ApplyPreview | null
}

export interface UndoReport {
  restored: number
  left_alone: { task: string; why: string }[]
}

export interface Criterion {
  text: string
  result: 'pass' | 'fail' | 'unclear'
  observation_ids?: string[]
  note?: string
}

export interface Observation {
  id: string
  kind: string
  fact: string
  ok: boolean | null
}

export interface VerdictView {
  verdict: 'pass' | 'fail' | 'inconclusive'
  level: string
  tier: number
  artifact: string
  verifier: string
  independence: string
  reasons: string
  criteria: Criterion[]
  created_at: string
}

export interface AuditCard {
  audit: { id: string; task_id: string; tier: number; source: string; verifier_config: string; selected_at: string }
  task: { id: string; display_id: string; title: string; status: string } | null
  verdict: VerdictView | null
  observations: Observation[]
  links: { kind: string; ref: string }[]
}

export interface AuditStack {
  drawn: number
  cards: AuditCard[]
  calibration: Calibration
}

export interface TaskVerdict extends VerdictView {
  id: string
  applied: boolean
  waiting_for: WaitingFor | null
  observations: Observation[]
}

export interface SecondOpinion {
  task_id: string
  display_id: string
  requested_at: string
  state: 'pending' | 'agreed' | 'disagreed' | 'failed'
  detail: string
  answered_at: string | null
}

export interface TaskVerification {
  verdicts: TaskVerdict[]
  waiting_for: { who: WaitingFor; kind: string; reason: string } | null
  hold: { release_at: number } | null
  second_opinion: SecondOpinion | null
}

const BASE = '/api/verification'

export const fetchVerificationStatus = () => hqJson<VerificationStatus>(`${BASE}/status`)
export const fetchQueueHealth = () => hqJson<QueueHealth>(`${BASE}/queue`)
export const fetchBriefing = (hours = 24) => hqJson<Briefing>(`${BASE}/briefing?hours=${encodeURIComponent(String(hours))}`)

export const listSweeps = () => hqJson<{ sweeps: Sweep[] }>(`${BASE}/sweeps`)
export const createSweep = (scope: SweepScope) => hqJson<Sweep>(`${BASE}/sweeps`, 'POST', { scope })
export const fetchSweep = (id: string) => hqJson<SweepDetail>(`${BASE}/sweeps/${encodeURIComponent(id)}`)

export type SweepAction = 'run' | 'pause' | 'cancel' | 'apply' | 'undo'

export function sweepAction(id: string, action: 'undo'): Promise<UndoReport>
export function sweepAction(id: string, action: Exclude<SweepAction, 'undo'>, confirm?: boolean): Promise<Sweep>
export function sweepAction(id: string, action: SweepAction, confirm = false): Promise<Sweep | UndoReport> {
  const body = action === 'apply' ? { confirm } : undefined
  return hqJson(`${BASE}/sweeps/${encodeURIComponent(id)}/${action}`, 'POST', body)
}

/** `n` draws that many more audits first, favouring what has the least data. */
export const fetchAudits = (n = 0) => hqJson<AuditStack>(`${BASE}/audits${n > 0 ? `?n=${n}` : ''}`)
export const answerAudit = (id: string, answer: 'agree' | 'disagree', note = '') =>
  hqJson<{ audit: unknown }>(`${BASE}/audits/${encodeURIComponent(id)}`, 'POST', { answer, note })

export const vetoTask = (taskId: string, note = '') =>
  hqJson<{ vetoed: boolean }>(`${BASE}/holds/${encodeURIComponent(taskId)}/veto`, 'POST', { note })

export const fetchTaskVerification = (taskId: string) =>
  hqJson<TaskVerification>(`/api/tasks/${encodeURIComponent(taskId)}/verification`)
export const requestSecondOpinion = (taskId: string) =>
  hqJson<SecondOpinion>(`/api/tasks/${encodeURIComponent(taskId)}/reverify`, 'POST')
