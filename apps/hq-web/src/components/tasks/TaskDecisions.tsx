import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { answerDecision, fetchTaskDecisions, type TaskDecision } from '~/lib/taskDecisionsApi'
import { useRefreshOn } from '~/lib/useRefreshOn'
import { usePolled } from '../sessions/usePolled'
import { SectionLabel } from './taskFields'

const DECISIONS_POLL_MS = 30_000

const ASKS: Record<string, string> = {
  command: 'wants to run',
  edit: 'wants to change',
  path: 'wants to reach',
  question: 'asks',
  trust: 'asks whether to trust',
}

/** The dialogs this task's agents wait on you for. Shown only while there are any. */
export function TaskDecisions({ taskId }: { taskId: string }) {
  const decisions = usePolled(taskId, () => fetchTaskDecisions(taskId), DECISIONS_POLL_MS)
  useRefreshOn(['session:changed', 'task:sync'], decisions.refresh)
  const open = decisions.data?.decisions ?? []
  if (open.length === 0) return null
  return (
    <section aria-label="Decisions waiting for you">
      <SectionLabel>Waiting for you</SectionLabel>
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
      setError(e instanceof Error ? e.message : 'Could not answer that.')
    } finally {
      await onAnswered()
      setBusy(false)
    }
  }
  return (
    <li className="px-2.5 py-2 rounded-lg border border-amber-500/20 bg-amber-500/5 space-y-1.5">
      <p className="text-xs text-neutral-300">
        <Link to="/sessions" search={{ id: d.session_id }} className="underline decoration-dotted">
          {d.session_id}
        </Link>{' '}
        {ASKS[d.kind] ?? 'needs an answer'}
        {d.question && d.kind === 'question' ? `: ${d.question}` : ''}
      </p>
      {d.command && (
        <pre className="max-h-28 overflow-auto rounded bg-black/40 px-2 py-1 text-[11px] text-neutral-200 whitespace-pre-wrap break-words">
          {d.command}
        </pre>
      )}
      <div className="flex flex-wrap gap-2">
        {d.approve && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void answer('approve')}
            className="h-8 px-3 rounded-lg border border-white/10 bg-white/5 text-xs font-semibold hover:bg-white/10 disabled:opacity-50"
            style={{ color: 'var(--accent-green)' }}
          >
            Allow once
          </button>
        )}
        <button
          type="button"
          disabled={busy}
          onClick={() => void answer('decline')}
          className="h-8 px-3 rounded-lg border border-white/10 bg-white/5 text-xs font-semibold hover:bg-white/10 disabled:opacity-50"
          style={{ color: 'var(--accent-red)' }}
        >
          Decline
        </button>
        <Link
          to="/sessions"
          search={{ id: d.session_id }}
          className="h-8 px-3 inline-flex items-center rounded-lg border border-white/10 text-xs text-neutral-300 hover:bg-white/5"
        >
          Other answers
        </Link>
      </div>
      {error && (
        <p role="alert" className="text-[11px] text-rose-400">
          {error}
        </p>
      )}
    </li>
  )
}
