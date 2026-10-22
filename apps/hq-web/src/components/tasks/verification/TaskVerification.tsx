import { useState } from 'react'
import { Check, ChevronDown, Hourglass, MinusCircle, Undo2, X } from 'lucide-react'
import {
  fetchTaskVerification,
  requestSecondOpinion,
  vetoTask,
  type TaskVerdict,
  type TaskVerification as TaskVerificationData,
} from '~/lib/verificationApi'
import type { TaskItem } from '~/lib/tasksApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../../sessions/usePolled'
import { SectionLabel } from '../taskFields'
import { WAITING_LABEL, countdown, criteriaSummary, levelLabel } from './verification'

const POLL_MS = 60_000
const OLDER_SHOWN = 4

const VERDICT_STYLE: Record<TaskVerdict['verdict'], string> = {
  pass: 'bg-emerald-400/15 text-emerald-300',
  fail: 'bg-rose-400/15 text-rose-300',
  inconclusive: 'bg-amber-400/15 text-amber-300',
}

function VerdictBlock({ v, open }: { v: TaskVerdict; open: boolean }) {
  return (
    <details open={open} className="rounded-xl border border-white/10 bg-white/[0.02] px-3 py-2">
      <summary className="flex items-center gap-2 cursor-pointer list-none min-h-8">
        <span className={`text-[11px] font-semibold px-1.5 py-0.5 rounded-md ${VERDICT_STYLE[v.verdict]}`}>{v.verdict}</span>
        <span className="text-xs text-neutral-300 min-w-0 flex-1 truncate">
          tier {v.tier}, {levelLabel(v.level)}
          {v.applied ? '' : ', not applied'}
        </span>
        <span className="text-[11px] text-neutral-500 shrink-0">{v.created_at.slice(0, 16)}</span>
        <ChevronDown className="w-3.5 h-3.5 text-neutral-500 shrink-0" />
      </summary>
      <div className="mt-2 space-y-2.5">
        <p className="text-xs text-neutral-400 break-words">
          {v.verifier}, {v.independence.replaceAll('_', ' ')}. Artifact: {v.artifact}
        </p>
        {v.reasons && <p className="text-xs text-neutral-300 whitespace-pre-wrap break-words">{v.reasons}</p>}
        <div>
          <p className="text-[11px] font-semibold text-neutral-500 uppercase tracking-wider mb-1">{criteriaSummary(v.criteria)}</p>
          <ul className="space-y-1">
            {v.criteria.map((c) => (
              <li key={c.text} className="flex items-start gap-1.5 text-xs text-neutral-200">
                {c.result === 'pass' ? (
                  <Check className="w-3.5 h-3.5 text-emerald-400 shrink-0 mt-0.5" aria-label="met" />
                ) : c.result === 'fail' ? (
                  <X className="w-3.5 h-3.5 text-rose-400 shrink-0 mt-0.5" aria-label="not met" />
                ) : (
                  <MinusCircle className="w-3.5 h-3.5 text-neutral-500 shrink-0 mt-0.5" aria-label="not decided" />
                )}
                <span className="break-words">{c.text}</span>
              </li>
            ))}
          </ul>
        </div>
        <ul className="space-y-1">
          {v.observations.map((o) => (
            <li key={o.id} className="flex items-start gap-1.5 text-xs text-neutral-400">
              {o.ok === true ? (
                <Check className="w-3.5 h-3.5 text-emerald-400 shrink-0 mt-0.5" />
              ) : o.ok === false ? (
                <X className="w-3.5 h-3.5 text-rose-400 shrink-0 mt-0.5" />
              ) : (
                <MinusCircle className="w-3.5 h-3.5 text-neutral-600 shrink-0 mt-0.5" />
              )}
              <span className="break-words">
                <span className="text-neutral-600">{o.id}</span> {o.fact}
              </span>
            </li>
          ))}
        </ul>
      </div>
    </details>
  )
}

interface ViewProps {
  data: TaskVerificationData
  taskStatus: TaskItem['status']
  nowMs: number
  busy: boolean
  /** The owner's two-step "complete anyway" is armed. */
  confirmingComplete: boolean
  onSecondOpinion: () => void
  onVeto: () => void
  onCompleteAnyway: () => void
  onReturnToDo: () => void
}

/** Verdict history and the evidence behind it, who the task waits on, and the owner's overrides. */
export function TaskVerificationView({ data, taskStatus, nowMs, busy, confirmingComplete, onSecondOpinion, onVeto, onCompleteAnyway, onReturnToDo }: ViewProps) {
  const [latest, ...older] = data.verdicts
  const opinion = data.second_opinion
  const pending = opinion?.state === 'pending'
  return (
    <div className="space-y-3">
      {data.waiting_for && (
        <p className={`text-xs ${data.waiting_for.who === 'owner' ? 'text-amber-300' : 'text-neutral-400'}`}>
          <span className="font-semibold">{WAITING_LABEL[data.waiting_for.who]}.</span> {data.waiting_for.reason}
        </p>
      )}

      {data.hold && (
        <div className="flex items-center gap-2 px-3 py-2 rounded-xl border border-amber-400/30 bg-amber-400/10">
          <Hourglass className="w-4 h-4 text-amber-300 shrink-0" />
          <span className="text-xs text-amber-100 flex-1">Tier 2: it completes in {countdown(data.hold.release_at, nowMs)} unless you veto it.</span>
          <button type="button" disabled={busy} onClick={onVeto} className="hq-btn-ghost !h-9 !px-3 !text-xs disabled:opacity-40">
            <Undo2 className="w-3.5 h-3.5" />
            Veto
          </button>
        </div>
      )}

      {latest ? (
        <div className="space-y-2">
          <VerdictBlock v={latest} open={taskStatus !== 'complete'} />
          {older.length > 0 && (
            <details>
              <summary className="text-xs text-neutral-500 cursor-pointer">{older.length} earlier {older.length === 1 ? 'verdict' : 'verdicts'}</summary>
              <div className="mt-2 space-y-2">
                {older.slice(0, OLDER_SHOWN).map((v) => (
                  <VerdictBlock key={v.id} v={v} open={false} />
                ))}
              </div>
            </details>
          )}
        </div>
      ) : (
        <p className="text-xs text-neutral-500">Not verified yet.</p>
      )}

      {opinion && (
        <p className="text-xs text-neutral-400">
          Second opinion:{' '}
          {opinion.state === 'pending'
            ? 'asked, waiting for another verifier.'
            : opinion.state === 'agreed'
              ? 'it agreed.'
              : opinion.state === 'disagreed'
                ? `it disagreed. ${opinion.detail}`
                : `it could not be done. ${opinion.detail}`}
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        {latest && (
          <button type="button" disabled={busy || pending} onClick={onSecondOpinion} className="hq-btn-ghost !h-10 !px-3.5 !text-xs disabled:opacity-40">
            {pending ? 'Second opinion asked' : 'Second opinion'}
          </button>
        )}
        {taskStatus !== 'complete' && (
          <button
            type="button"
            disabled={busy}
            onClick={onCompleteAnyway}
            className={`!h-10 !px-3.5 !text-xs disabled:opacity-40 ${confirmingComplete ? 'hq-btn-primary' : 'hq-btn-ghost'}`}
          >
            {confirmingComplete ? 'Tap again to complete it' : 'Complete anyway'}
          </button>
        )}
        {taskStatus !== 'to_do' && (
          <button type="button" disabled={busy} onClick={onReturnToDo} className="hq-btn-ghost !h-10 !px-3.5 !text-xs disabled:opacity-40">
            Return to to_do
          </button>
        )}
      </div>
    </div>
  )
}

interface Props {
  task: TaskItem
  /** The page's own task update, so the board sees the change at once. */
  onUpdate: (id: string, patch: Record<string, unknown>) => Promise<void>
}

/** Hidden for a task verification has nothing to say about: never verified and not waiting on it. */
export function TaskVerification({ task, onUpdate }: Props) {
  const found = usePolled(`task-verification-${task.id}`, () => fetchTaskVerification(task.id), POLL_MS)
  useRefreshOn(['task:sync'], found.refresh)
  const [busy, setBusy] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const data = found.data
  if (!data) return null
  const relevant = data.verdicts.length > 0 || data.hold !== null || data.second_opinion !== null || (data.waiting_for !== null && data.waiting_for.kind !== 'review')
  if (!relevant) return null

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true)
    try {
      await fn()
      await found.refresh()
    } finally {
      setBusy(false)
      setConfirming(false)
    }
  }

  return (
    <div>
      <SectionLabel className="mb-2">Verification</SectionLabel>
      <TaskVerificationView
        data={data}
        taskStatus={task.status}
        nowMs={Date.now()}
        busy={busy}
        confirmingComplete={confirming}
        onSecondOpinion={() => run(() => requestSecondOpinion(task.id))}
        onVeto={() => run(() => vetoTask(task.id, 'vetoed from the task'))}
        onCompleteAnyway={() => (confirming ? run(() => onUpdate(task.id, { status: 'complete', expected_status: task.status })) : setConfirming(true))}
        onReturnToDo={() => run(() => onUpdate(task.id, { status: 'to_do', expected_status: task.status }))}
      />
    </div>
  )
}
