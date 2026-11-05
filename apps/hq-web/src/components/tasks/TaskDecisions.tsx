import { ApprovalCard } from '../ui/ApprovalCard'
import { t, type StringKey } from '~/strings'
import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { answerDecision, fetchTaskDecisions, type TaskDecision } from '~/lib/taskDecisionsApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'

const DECISIONS_POLL_MS = 30_000

const ASKS: Record<string, StringKey> = {
  command: 'task_dec_runs',
  edit: 'task_dec_changes',
  path: 'task_dec_reaches',
  question: 'task_dec_asks',
  trust: 'task_dec_trust',
}

/** The dialogs this task's agents wait on you for. Shown only while there are any. */
export function TaskDecisions({ taskId }: { taskId: string }) {
  const decisions = usePolled(taskId, () => fetchTaskDecisions(taskId), DECISIONS_POLL_MS, true, true, ['task:sync'])
  useRefreshOn(['session:changed', 'task:sync'], decisions.refresh)
  const open = decisions.data?.decisions ?? []
  if (open.length === 0) return null
  return (
    <section aria-label={t('task_dec_aria')}>
      <SectionLabel>{t('task_dec_title')}</SectionLabel>
      <ul className="space-y-2">
        {open.map((d) => (
          <DecisionRow key={d.item_id} decision={d} onAnswered={decisions.refresh} />
        ))}
      </ul>
    </section>
  )
}

function DecisionRow({ decision: d, onAnswered }: { decision: TaskDecision; onAnswered: () => Promise<void> }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const answer = async (choice: 'approve' | 'decline') => {
    setBusy(true)
    setError(null)
    try {
      await answerDecision(d.notification_id, choice)
    } catch (e) {
      setError(e instanceof Error ? e.message : t('task_dec_error'))
    } finally {
      await onAnswered()
      setBusy(false)
    }
  }
  return (
    <li className="px-2.5 py-2 rounded-lg border border-amber-500/20 bg-amber-500/5 space-y-1.5">
      <p className="text-xs text-neutral-300">
        <Link to="/helpers/$id" params={{ id: d.session_id }} className="underline decoration-dotted">
          {d.session_id}
        </Link>{' '}
        {t(ASKS[d.kind] ?? 'task_dec_needs')}
        {d.question ? `: ${d.question}` : ''}
      </p>
      <ApprovalCard
        quoteTitle={t('approval_quote_task', { helper: d.session_id })}
        command={d.command}
        loses={d.loses}
        verb={d.verb}
        canAllow={!!d.approve}
        busy={busy}
        error={error}
        onDecline={() => void answer('decline')}
        onAllow={() => void answer('approve')}
        more={
          <Link to="/helpers/$id" params={{ id: d.session_id }} className="a-btn secondary">
            {t('approval_other_answers')}
          </Link>
        }
      />
    </li>
  )
}
