import { useState } from 'react'
import { ChevronDown, Loader2, Pause, Play, RotateCcw, X } from 'lucide-react'
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
  { id: 'initiative', label: 'This initiative' },
  { id: 'since', label: 'Since a date' },
  { id: 'selected', label: 'The tasks I picked' },
]

export function ScopePickerView({ inputs, initiativeName, choice, onChoice, onSince, busy, error, recent, onStart, onOpen }: PickerProps) {
  const scope = pickScope(choice, inputs)
  const missing =
    choice === 'initiative' && !inputs.initiativeId
      ? 'Pick an initiative in the sidebar first.'
      : choice === 'selected' && inputs.selectedIds.length === 0
        ? 'Tick some tasks with Select first.'
        : choice === 'since' && !scope
          ? 'Give a date as YYYY-MM-DD.'
          : null
  return (
    <div className="space-y-4">
      <p className="text-xs text-neutral-400">
        A sweep reads the evidence for each task waiting for review, with no model and no spend, and sorts them into groups. Nothing moves until you read the groups and apply.
      </p>
      <fieldset className="space-y-2">
        <legend className="sr-only">What to sweep</legend>
        {CHOICES.map((c) => (
          <label
            key={c.id}
            className={`flex items-center gap-3 min-h-11 px-3.5 rounded-xl border cursor-pointer ${choice === c.id ? 'border-emerald-400/50 bg-emerald-400/10' : 'border-white/10 bg-white/[0.02]'}`}
          >
            <input type="radio" name="sweep-scope" checked={choice === c.id} onChange={() => onChoice(c.id)} className="accent-emerald-400" />
            <span className="text-sm text-neutral-100">
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
          className="min-h-11 px-3 rounded-xl text-sm text-neutral-100 hq-field"
        />
      )}
      {missing && <p className="text-xs text-amber-300">{missing}</p>}
      {error && <p className="text-xs text-rose-300">{error}</p>}
      <button type="button" disabled={busy || !scope} onClick={onStart} className={`${SHEET_PRIMARY} w-full`}>
        {busy ? <Loader2 className="w-4 h-4 animate-spin" /> : null}
        Make the plan
      </button>
      {recent.length > 0 && (
        <div>
          <h3 className="text-[11px] font-semibold text-neutral-500 uppercase tracking-wider mb-1.5">Earlier sweeps</h3>
          <ul className="space-y-1.5">
            {recent.slice(0, 5).map((s) => (
              <li key={s.id}>
                <button type="button" onClick={() => onOpen(s.id)} className="w-full text-left min-h-11 px-3 py-2 rounded-xl bg-white/[0.03] border border-white/10 hover:bg-white/[0.06]">
                  <span className="block text-xs text-neutral-100">{scopeLabel(s.scope)}</span>
                  <span className="block text-[11px] text-neutral-500">
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
    <ul className="mt-2 space-y-1">
      {shown.map((i) => (
        <li key={i.task_id} className="text-xs text-neutral-400 break-words">
          <span className="text-neutral-200">{i.display_id}</span> {i.title}
          {i.finding ? <span className="text-neutral-500"> · {i.finding}</span> : null}
          {i.state === 'stale' || i.state === 'kept' ? <span className="text-amber-300/80"> · {i.detail}</span> : null}
        </li>
      ))}
      {items.length > shown.length && <li className="text-[11px] text-neutral-500">and {items.length - shown.length} more</li>}
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
        {Icon ? <Icon className="w-4 h-4" /> : null}
        {label}
      </button>
    )
  }
  return (
    <div className="space-y-4">
      <div>
        <p className="text-xs text-neutral-500">{scopeLabel(sweep.scope, initiativeName)}</p>
        <p className="text-sm font-semibold text-white flex items-center gap-2">
          {live && <Loader2 className="w-4 h-4 animate-spin text-emerald-400" />}
          {stage.label}
        </p>
        <p className="text-xs text-neutral-400 mt-0.5">{stage.hint}</p>
      </div>

      {bar && (
        <div role="progressbar" aria-valuenow={bar.percent} aria-valuemin={0} aria-valuemax={100} aria-label={bar.label}>
          <div className="h-2 rounded-full bg-white/10 overflow-hidden">
            <div className="h-full bg-emerald-400 transition-all" style={{ width: `${bar.percent}%` }} />
          </div>
          <p className="text-[11px] text-neutral-500 mt-1">{bar.label}</p>
        </div>
      )}

      {rows.length > 0 && (
        <ul className="space-y-2">
          {rows.map((r) => {
            const mine = items.filter((i) => i.bucket === r.bucket)
            return (
              <li key={r.bucket} className="rounded-xl border border-white/10 bg-white/[0.02] px-3.5 py-2.5">
                <details>
                  <summary className="flex items-baseline justify-between gap-3 cursor-pointer min-h-8 list-none">
                    <span className="text-sm text-neutral-100">
                      <span className="font-semibold text-white">{r.count}</span> {r.label.toLowerCase()}
                    </span>
                    <span className="text-[11px] text-neutral-500 text-right">{BUCKET_INFO[r.bucket].does}</span>
                    <ChevronDown className="w-3.5 h-3.5 text-neutral-500 shrink-0 self-center" aria-hidden="true" />
                  </summary>
                  <ItemList items={mine} />
                </details>
              </li>
            )
          })}
        </ul>
      )}

      {sweep.plan.would_wait_on_you !== undefined && sweep.plan.would_wait_on_you > 0 && (
        <p className="text-xs text-neutral-400">{sweep.plan.would_wait_on_you} would wait on you, including tier 2 tasks, which get a veto window.</p>
      )}
      {estimate && <p className="text-xs text-neutral-400">{estimate}</p>}

      {needsConfirm && preview && (
        <label className="flex items-start gap-3 px-3.5 py-3 rounded-xl border border-amber-400/30 bg-amber-400/10 cursor-pointer">
          <input type="checkbox" checked={confirmed} onChange={(e) => onConfirm(e.target.checked)} className="mt-0.5 accent-amber-400 w-4 h-4" />
          <span className="text-xs text-amber-100">
            This moves {preview.will_move} tasks, more than the {preview.confirm_over} that HQ will apply without asking. I have read the groups and want it applied.
          </span>
        </label>
      )}

      {undone && (
        <div className="rounded-xl border border-white/10 bg-white/[0.03] px-3.5 py-3 text-xs space-y-1">
          <p className="text-neutral-100">Restored {undone.restored} tasks to review.</p>
          {undone.left_alone.map((l) => (
            <p key={l.task} className="text-neutral-400">
              Left alone: {l.task}, {l.why}
            </p>
          ))}
        </div>
      )}

      {error && <p className="text-xs text-rose-300">{error}</p>}
      {stage.actions.length > 0 && <div className="flex flex-wrap gap-2">{stage.actions.map(button)}</div>}
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
    <Sheet title={sweepId ? 'Sweep' : 'Verify now'} onClose={close}>
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
        <p className="text-sm text-neutral-400 flex items-center gap-2">
          <Loader2 className="w-4 h-4 animate-spin" />
          {detail.error ?? 'Loading the sweep'}
        </p>
      )}
    </Sheet>
  )
}
