import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import type { NotificationItem, NotificationDialog } from '~/lib/notificationsApi'
import { answerDecision } from '~/lib/taskDecisionsApi'

const BUTTON_CLASS =
  'h-8 px-3 rounded-lg border border-white/10 bg-white/5 text-xs font-semibold hover:bg-white/10 disabled:opacity-50'

/** The dialog an agent waits at, when this notice is about one nobody has answered yet. */
export function openDialog(n: NotificationItem): NotificationDialog | null {
  const d = n.metadata.dialog
  return n.state === 'pending' && d && !d.answered ? d : null
}

/**
 * Answers the agent's dialog itself, never the notice: Allow once only when the server named a plain
 * yes, Decline always, and every other answer on the agent's own page, where each option is a button.
 */
export function DialogAnswer({
  notification,
  dialog,
  onAnswered,
}: {
  notification: NotificationItem
  dialog: NotificationDialog
  onAnswered: (id: string, choice: 'approve' | 'decline') => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const answer = async (choice: 'approve' | 'decline') => {
    setBusy(true)
    setError(null)
    try {
      await answerDecision(notification.id, choice)
      onAnswered(notification.id, choice)
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Could not answer that.')
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="space-y-1.5 w-full">
      {dialog.question && <p className="text-xs text-neutral-200">{dialog.question}</p>}
      {dialog.command && (
        <pre className="max-h-28 overflow-auto rounded bg-black/40 px-2 py-1 text-[11px] text-neutral-200 whitespace-pre-wrap break-words">
          {dialog.command}
        </pre>
      )}
      <div className="flex flex-wrap justify-end gap-2">
        {dialog.approve && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void answer('approve')}
            className={BUTTON_CLASS}
            style={{ color: 'var(--accent-green)' }}
          >
            Allow once
          </button>
        )}
        <button
          type="button"
          disabled={busy}
          onClick={() => void answer('decline')}
          className={BUTTON_CLASS}
          style={{ color: 'var(--accent-red)' }}
        >
          Decline
        </button>
        <Link
          to="/sessions"
          search={{ id: dialog.session_id }}
          className="h-8 px-3 inline-flex items-center rounded-lg border border-white/10 text-xs text-neutral-300 hover:bg-white/5"
        >
          {dialog.approve ? 'Other answers' : 'Answer on its page'}
        </Link>
      </div>
      {error && (
        <p role="alert" className="text-[11px] text-rose-400">
          {error}
        </p>
      )}
    </div>
  )
}
