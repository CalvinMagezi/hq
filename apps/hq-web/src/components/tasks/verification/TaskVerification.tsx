import { useState } from 'react'
import { t, tn } from '~/strings'
import { Check, ChevronDown, Hourglass, MinusCircle, TriangleAlert, Undo2 } from 'lucide-react'
import '../leaf.css'
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
import { WAITING_LABEL, countdown, criteriaSummary, kindLabel, levelLabel } from './verification'

const POLL_MS = 60_000
const OLDER_SHOWN = 4

const VERDICT_WORD: Record<TaskVerdict['verdict'], string> = {
  pass: t('verify_met').replace('.', ''),
  fail: t('verify_not_met').replace('.', ''),
  inconclusive: t('check_not_decided_label'),
}

const VERDICT_STYLE: Record<TaskVerdict['verdict'], string> = {
  pass: 'a-kind sm',
  fail: 'a-state needs',
  inconclusive: 'a-kind sm',
}

function VerdictBlock({ v, open }: { v: TaskVerdict; open: boolean }) {
  return (
    <details open={open} className="l-details">
      <summary className="l-row">
        <span className={VERDICT_STYLE[v.verdict]}>{VERDICT_WORD[v.verdict]}</span>
        <span className="a-body-sm l-grow l-clip">
          {kindLabel(v.tier)}, {levelLabel(v.level)}
          {v.applied ? '' : `, ${t('check_not_applied')}`}
        </span>
        <span className="a-cap a-muted shrink-0">{v.created_at.slice(0, 16)}</span>
        <ChevronDown className="a-ic xs a-muted" aria-hidden="true" />
      </summary>
      <div className="l-stack mt-2">
        <p className="a-cap a-muted l-break">
          {t('check_who_made', { verifier: v.verifier, how: v.independence.replaceAll('_', ' ') })}
        </p>
        {v.reasons && <p className="a-body-sm l-pre">{v.reasons}</p>}
        <div>
          <p className="a-label mb-1">{criteriaSummary(v.criteria)}</p>
          <ul className="l-stack">
            {v.criteria.map((c) => (
              <li key={c.text} className="l-row a-body-sm">
                {c.result === 'pass' ? (
                  <Check className="a-ic xs" aria-label={t('verify_met')} />
                ) : c.result === 'fail' ? (
                  <TriangleAlert className="a-ic xs a-warn-t" aria-label={t('verify_not_met')} />
                ) : (
                  <MinusCircle className="a-ic xs a-muted" aria-label={t('verify_not_decided')} />
                )}
                <span className="l-break">{c.text}</span>
              </li>
            ))}
          </ul>
        </div>
        <ul className="l-stack">
          {v.observations.map((o) => (
            <li key={o.id} className="l-row a-cap a-muted">
              {o.ok === true ? (
                <Check className="a-ic xs" aria-hidden="true" />
              ) : o.ok === false ? (
                <TriangleAlert className="a-ic xs a-warn-t" aria-hidden="true" />
              ) : (
                <MinusCircle className="a-ic xs a-muted" aria-hidden="true" />
              )}
              <span className="l-break">
                <span className="a-mono">{o.id}</span> {o.fact}
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
    <div className="a-card" data-check-card>
      {data.waiting_for && (
        <p className={`a-body-sm ${data.waiting_for.who === 'owner' ? 'a-warn-t' : 'a-muted'}`}>
          <span className="font-semibold">{WAITING_LABEL[data.waiting_for.who]}.</span> {data.waiting_for.reason}
        </p>
      )}

      {data.hold && (
        <div className="l-row">
          <Hourglass className="a-ic sm a-warn-t" aria-hidden="true" />
          <span className="a-body-sm l-grow">{t('check_holds', { time: countdown(data.hold.release_at, nowMs) })}</span>
          <button type="button" disabled={busy} onClick={onVeto} className="a-btn secondary compact">
            <Undo2 className="a-ic xs" aria-hidden="true" />
            {t('check_keep_open')}
          </button>
        </div>
      )}

      {latest ? (
        <div className="l-stack">
          <VerdictBlock v={latest} open={taskStatus !== 'complete'} />
          {older.length > 0 && (
            <details className="l-details">
              <summary>{tn('check_earlier', older.length)}</summary>
              <div className="l-stack mt-2">
                {older.slice(0, OLDER_SHOWN).map((v) => (
                  <VerdictBlock key={v.id} v={v} open={false} />
                ))}
              </div>
            </details>
          )}
        </div>
      ) : (
        <p className="a-body-sm a-muted">{t('check_not_checked')}</p>
      )}

      {opinion && (
        <p className="a-body-sm a-muted">
          {opinion.state === 'pending'
            ? t('check_second_look_pending')
            : opinion.state === 'agreed'
              ? t('check_second_look_agreed')
              : opinion.state === 'disagreed'
                ? t('check_second_look_disagreed', { detail: opinion.detail })
                : t('check_second_look_failed', { detail: opinion.detail })}
        </p>
      )}

            <div className="l-bar">
        {taskStatus !== 'to_do' && (
          <button type="button" disabled={busy} onClick={onReturnToDo} className="a-btn secondary compact">
            {t('tasks_send_back')}
          </button>
        )}
        {latest && (
          <button type="button" disabled={busy || pending} onClick={onSecondOpinion} className="a-btn quiet compact">
            {pending ? t('check_second_look_asked') : t('check_second_look')}
          </button>
        )}
        {taskStatus !== 'complete' && (
          <button type="button" disabled={busy} onClick={onCompleteAnyway} className="a-btn compact l-end">
            {confirmingComplete ? t('check_mark_done_again') : t('tasks_mark_done')}
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
  const found = usePolled(`task-verification-${task.id}`, () => fetchTaskVerification(task.id), POLL_MS, true, true, ['task:sync'])
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
      <SectionLabel className="mb-2">{t('check_ready_to_check')}</SectionLabel>
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
