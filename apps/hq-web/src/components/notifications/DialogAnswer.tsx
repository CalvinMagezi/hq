import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import type { NotificationItem, NotificationDialog } from '~/lib/notificationsApi'
import { ApprovalCard } from '~/components/ui/ApprovalCard'
import { t } from '~/strings'
import { answerDecision } from '~/lib/taskDecisionsApi'

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
      setError(e instanceof Error ? e.message : t('fail_answer_refused'))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className="w-full">
      <ApprovalCard
        quoteTitle={t('approval_quote_inbox')}
        command={dialog.command}
        inWords={dialog.question}
        loses={dialog.loses}
        verb={dialog.verb}
        canAllow={!!dialog.approve}
        busy={busy}
        error={error}
        onDecline={() => void answer('decline')}
        onAllow={() => void answer('approve')}
        more={
          <Link
            to="/helpers/$id"
            params={{ id: dialog.session_id }}
            className="a-btn secondary"
          >
            {dialog.approve ? t('approval_other_answers') : t('approval_answer_on_page')}
          </Link>
        }
      />
    </div>
  )
}
