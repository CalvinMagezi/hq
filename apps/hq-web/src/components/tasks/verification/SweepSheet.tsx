import { t } from '~/strings'
import '../leaf.css'
import { useState } from 'react'
import { AlertTriangle, ChevronDown, Loader2, Pause, Play, RotateCcw, X } from 'lucide-react'
import {
  createSweep,
  fetchSweep,
  listSweeps,
  sweepAction,
  type Sweep,
  type SweepAction,
  type SweepDetail,
  type SweepItem,
  type UndoReport,
} from '~/lib/verificationApi'
import { usePolled } from '../../sessions/usePolled'
import { SHEET_DANGER, SHEET_PRIMARY, SHEET_QUIET, Sheet } from './Sheet'
import {
  BUCKET_INFO,
  bucketRows,
  estimateLine,
  pickScope,
  scopeLabel,
  sweepProgress,
  sweepStage,
  type ScopeChoice,
  type ScopeInputs,
} from './verification'

/** Items listed under a group; the count still covers all of them. */
const ITEMS_SHOWN = 6
const LIVE_POLL_MS = 3_000
const IDLE_POLL_MS = 20_000

// ---- choosing what to sweep ----

interface PickerProps {
  inputs: ScopeInputs
  initiativeName?: string
  choice: ScopeChoice
  onChoice: (c: ScopeChoice) => void
  onSince: (date: string) => void
  busy: boolean
  error: string | null
  recent: Sweep[]
  onStart: () => void
  onOpen: (id: string) => void
}

const CHOICES: { id: ScopeChoice; label: string }[] = [
  { id: 'all', label: 'Everything waiting' },
  { id: 'initiative', label: t('tasks_this_list') },
  { id: 'since', label: 'Since a date' },
  { id: 'selected', label: 'The tasks I picked' },
]

export function ScopePickerView({ inputs, initiativeName, choice, onChoice, onSince, busy, error, recent, onStart, onOpen }: PickerProps) {
  const scope = pickScope(choice, inputs)
  const missing =
    choice === 'initiative' && !inputs.initiativeId
      ? t('tasks_pick_list_first')
      : choice === 'selected' && inputs.selectedIds.length === 0
        ? 'Tick some tasks with Select first.'
        : choice === 'since' && !scope
          ? 'Give a date as YYYY-MM-DD.'
          : null
  return (
    <div className="l-stack">
      <p className="a-body-sm a-muted">
        A sweep reads the evidence for each task waiting for review, with no model and no spend, and sorts them into groups. Nothing moves until you read the groups and apply.
      </p>
      <fieldset className="l-stack">
        <legend className="sr-only">What to sweep</legend>
        {CHOICES.map((c) => (
          <label
            key={c.id}
            className={`l-choice ${choice === c.id ? 'is-on' : ''}`}
          >
            <input type="radio" name="sweep-scope" checked={choice === c.id} onChange={() => onChoice(c.id)} />
            <span className="a-body-sm">
              {c.id === 'initiative' && initiativeName ? `${c.label}: ${initiativeName}` : c.id === 'selected' ? `${c.label} (${inputs.selectedIds.length})` : c.label}
            </span>
          </label>
        ))}
      </fieldset>
      {choice === 'since' && (
        <input
          type="date"
          value={inputs.since}
          onChange={(e) => onSince(e.target.value)}
          aria-label="Waiting since"
          className="a-input"
        />
      )}
      {missing && (
        <p className="a-why">
          <AlertTriangle className="a-ic xs" aria-hidden="true" />
          {missing}
        </p>
      )}
      {error && <p role="alert" className="a-body-sm a-danger-t">{error}</p>}
      <button type="button" disabled={busy || !scope} onClick={onStart} className={`${SHEET_PRIMARY} w-full`}>
        {busy ? <Loader2 className="a-ic sm animate-spin" aria-hidden="true" /> : null}
        Make the plan
      </button>
      {recent.length > 0 && (
        <div>
          <h3 className="a-label mb-2">Earlier sweeps</h3>
          <ul className="a-card l-lines">
            {recent.slice(0, 5).map((s) => (
              <li key={s.id}>
                <button type="button" onClick={() => onOpen(s.id)} className="a-drow l-tall l-col l-link w-full">
                  <span className="block a-body-sm">{scopeLabel(s.scope)}</span>
                  <span className="block a-cap a-muted">
                    {s.status} · {s.plan.total ?? 0} tasks · {s.created_at}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}

// ---- reading and controlling one sweep ----

interface BodyProps {
  detail: SweepDetail
  initiativeName?: string
  busy: boolean
  error: string | null
  /** The person has ticked the confirmation an over-threshold apply needs. */
  confirmed: boolean
  onConfirm: (v: boolean) => void
  onAction: (a: SweepAction) => void
  undone: UndoReport | null
}

function ItemList({ items }: { items: SweepItem[] }) {
  const shown = items.slice(0, ITEMS_SHOWN)
  return (
    <ul className="l-stack mt-2">
      {shown.map((i) => (
        <li key={i.task_id} className="a-body-sm a-muted l-break">
          <b>{i.display_id}</b> {i.title}
          {i.finding ? <span className="a-muted"> · {i.finding}</span> : null}
          {i.state === 'stale' || i.state === 'kept' ? <span className="a-warn-t"> · {i.detail}</span> : null}
        </li>
      ))}
      {items.length > shown.length && <li className="a-cap a-muted">and {items.length - shown.length} more</li>}
    </ul>
  )
}

export function SweepBodyView({ detail, initiativeName, busy, error, confirmed, onConfirm, onAction, undone }: BodyProps) {
  const { sweep, progress, items, apply_preview: preview } = detail
  const stage = sweepStage(sweep.status, preview)
  const bar = sweepProgress(sweep.status, progress)
  const rows = bucketRows(progress.buckets)
  const estimate = estimateLine(sweep.plan.estimate)
  const needsConfirm = sweep.status === 'ready' && preview?.needs_confirmation === true
  const live = sweep.status === 'planning' || sweep.status === 'running' || sweep.status === 'applying'
  const button = (a: SweepAction) => {
    const apply = a === 'apply'
    const label =
      a === 'run' ? (sweep.status === 'paused' ? 'Continue' : 'Run it') : a === 'pause' ? 'Pause' : a === 'cancel' ? 'Cancel' : a === 'apply' ? `Apply${preview ? ` (${preview.will_move})` : ''}` : 'Undo'
    const Icon = a === 'run' ? Play : a === 'pause' ? Pause : a === 'cancel' ? X : a === 'undo' ? RotateCcw : null
    const cls = apply || a === 'run' ? SHEET_PRIMARY : a === 'cancel' ? SHEET_DANGER : SHEET_QUIET
    return (
      <button key={a} type="button" disabled={busy || (apply && needsConfirm && !confirmed)} onClick={() => onAction(a)} className={`${cls} flex-1`}>
        {Icon ? <Icon className="a-ic sm" aria-hidden="true" /> : null}
        {label}
      </button>
    )
  }
  return (
    <div className="l-stack">
      <div>
        <p className="a-cap a-muted">{scopeLabel(sweep.scope, initiativeName)}</p>
        <p className="a-body-sm-b l-row">
          {live && <Loader2 className="a-ic sm animate-spin" aria-hidden="true" />}
          {stage.label}
        </p>
        <p className="a-body-sm a-muted">{stage.hint}</p>
      </div>

      {bar && (
        <div role="progressbar" aria-valuenow={bar.percent} aria-valuemin={0} aria-valuemax={100} aria-label={bar.label}>
          <div className="a-prog blue">
            <u style={{ width: `${bar.percent}%` }} />
          </div>
          <p className="a-cap a-muted mt-1">{bar.label}</p>
        </div>
      )}

      {rows.length > 0 && (
        <ul className="a-card l-lines">
          {rows.map((r) => {
            const mine = items.filter((i) => i.bucket === r.bucket)
            return (
              <li key={r.bucket} className="a-drow l-tall l-col">
                <details className="l-details">
                  <summary className="l-row">
                    <span className="a-body-sm">
                      <b>{r.count}</b> {r.label.toLowerCase()}
                    </span>
                    <span className="a-cap a-muted">{BUCKET_INFO[r.bucket].does}</span>
                    <ChevronDown className="a-ic xs a-muted" aria-hidden="true" />
                  </summary>
                  <ItemList items={mine} />
                </details>
              </li>
            )
          })}
        </ul>
      )}

      {sweep.plan.would_wait_on_you !== undefined && sweep.plan.would_wait_on_you > 0 && (
        <p className="a-body-sm a-muted">{sweep.plan.would_wait_on_you} would wait on you, including risky ones, which wait a while before HQ marks them done.</p>
      )}
      {estimate && <p className="a-body-sm a-muted">{estimate}</p>}

      {needsConfirm && preview && (
        <label className="a-card needs l-row">
          <input type="checkbox" checked={confirmed} onChange={(e) => onConfirm(e.target.checked)} />
          <span className="a-body-sm">
            This moves {preview.will_move} tasks, more than the {preview.confirm_over} that HQ will apply without asking. I have read the groups and want it applied.
          </span>
        </label>
      )}

      {undone && (
        <div className="a-card a-body-sm">
          <p>Restored {undone.restored} tasks to review.</p>
          {undone.left_alone.map((l) => (
            <p key={l.task} className="a-muted">
              Left alone: {l.task}, {l.why}
            </p>
          ))}
        </div>
      )}

      {error && <p role="alert" className="a-body-sm a-danger-t">{error}</p>}
      {stage.actions.length > 0 && <div className="l-bar">{stage.actions.map(button)}</div>}
    </div>
  )
}

// ---- the sheet ----

interface Props {
  open: boolean
  onClose: () => void
  /** What the page already knows, so the picker can offer it. */
  inputs: Omit<ScopeInputs, 'since'>
  initiativeName?: string
  /** Opened on a sweep that exists already. */
  initialSweepId?: string | null
}

const errorText = (e: unknown) => (e instanceof Error ? e.message : 'Something went wrong')

export function SweepSheet({ open, onClose, inputs, initiativeName, initialSweepId = null }: Props) {
  const [sweepId, setSweepId] = useState<string | null>(initialSweepId)
  const [choice, setChoice] = useState<ScopeChoice>(inputs.selectedIds.length > 0 ? 'selected' : 'all')
  const [since, setSince] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [confirmed, setConfirmed] = useState(false)
  const [undone, setUndone] = useState<UndoReport | null>(null)

  const recent = usePolled('verification-sweeps', async () => (await listSweeps()).sweeps, IDLE_POLL_MS, open && sweepId === null)
  const detail = usePolled(
    `verification-sweep-${sweepId ?? 'none'}`,
    () => fetchSweep(sweepId ?? ''),
    LIVE_POLL_MS,
    open && sweepId !== null
  )
  if (!open) return null

  const guard = async (fn: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await fn()
    } catch (e) {
      setError(errorText(e))
    } finally {
      setBusy(false)
    }
  }

  const start = () =>
    guard(async () => {
      const scope = pickScope(choice, { ...inputs, since })
      if (!scope) return
      const sweep = await createSweep(scope)
      setUndone(null)
      setConfirmed(false)
      setSweepId(sweep.id)
    })

  const act = (action: SweepAction) =>
    guard(async () => {
      if (!sweepId) return
      if (action === 'undo') setUndone(await sweepAction(sweepId, 'undo'))
      else await sweepAction(sweepId, action, confirmed)
      await detail.refresh()
    })

  const close = () => {
    setSweepId(initialSweepId)
    setUndone(null)
    setConfirmed(false)
    setError(null)
    onClose()
  }

  return (
    <Sheet title={sweepId ? 'Sweep' : t('check_sweep_title')} onClose={close}>
      {sweepId === null ? (
        <ScopePickerView
          inputs={{ ...inputs, since }}
          initiativeName={initiativeName}
          choice={choice}
          onChoice={setChoice}
          onSince={setSince}
          busy={busy}
          error={error}
          recent={recent.data ?? []}
          onStart={start}
          onOpen={setSweepId}
        />
      ) : detail.data ? (
        <SweepBodyView
          detail={detail.data}
          initiativeName={initiativeName}
          busy={busy}
          error={error ?? detail.error}
          confirmed={confirmed}
          onConfirm={setConfirmed}
          onAction={act}
          undone={undone}
        />
      ) : (
        <p className="a-loading">
          <Loader2 className="a-ic sm animate-spin" aria-hidden="true" />
          {detail.error ?? 'Loading the sweep'}
        </p>
      )}
    </Sheet>
  )
}
