import { ApprovalCard } from '~/components/ui/ApprovalCard'
import { t } from '~/strings'
import { useState, type ComponentType } from 'react'
import { Link } from '@tanstack/react-router'
import { answerErrorLine, cardKind, lapsed, serverHqLine, type CardKind } from '~/lib/inboxCard'
import { ClipboardCheck, LifeBuoy, ShieldAlert, Sparkles, TriangleAlert } from 'lucide-react'
import type { NotificationItem } from '~/lib/notificationsApi'
import { relTime } from '~/lib/time'
import { DialogAnswer, openDialog } from './DialogAnswer'
import { SignInActions, SignInBody } from './SignInItem'
import { useQuery } from '@tanstack/react-query'
import { listDevices, signOutDevice } from '~/lib/deviceSignIn'
import { toastAlert } from '~/lib/toast'
import { clockTime } from './UpdateRow'

interface Props {
  notification: NotificationItem
  selected?: boolean
  onAction(id: string, action: string): Promise<void>
  onAnswered: (id: string, choice: 'approve' | 'decline') => void
  loadingAction: string | null
}

const KIND_ICON: Record<CardKind, ComponentType<{ className?: string; 'aria-hidden'?: 'true' }>> = {
  sign_in: ShieldAlert,
  question: Sparkles,
  approval: Sparkles,
  task_check: ClipboardCheck,
  stuck: LifeBuoy,
  update: Sparkles,
}

/** One Needs you item: who asks, where and how long ago, what it wants, then Decline on the left and the answer on the right. */
export function NotificationCard({ notification, selected, onAction, onAnswered, loadingAction }: Props) {
  const isPending = notification.state === 'pending'
  const [failure, setFailure] = useState<string | null>(null)
  const [refused, setRefused] = useState(false)
  const [sent, setSent] = useState<{ answer: string; at: Date } | null>(null)
  const kind = cardKind(notification)
  const ranOut = lapsed(notification)
  const stuckOpen = kind === 'stuck' && isPending && !ranOut
  const act = async (id: string, action: string) => {
    setFailure(null)
    setRefused(false)
    try {
      await onAction(id, action)
      if (action === 'approve' || action === 'reject') setSent({ answer: t(action === 'approve' ? 'inbox_answer_allowed' : 'inbox_answer_declined'), at: new Date() })
    } catch (e) {
      setFailure(answerErrorLine(e))
      setRefused((e as { status?: number } | null)?.status === 409)
    }
  }
  const dialog = openDialog(notification)
  const signInId = notification.metadata.sign_in?.device_id
  const devices = useQuery({ queryKey: ['auth-devices'], queryFn: listDevices, enabled: !!signInId && isPending })
  const device = devices.data?.devices.find((d) => d.id === signInId)?.label ?? null
  const helperId = notification.metadata.extra?.session_id ?? notification.id
  const Icon = KIND_ICON[kind]
  const where = [serverHqLine(notification), relTime(notification.created_at)].filter(Boolean).join(' · ')
  const approvable = notification.actions.includes('approve')

  const signInActions =
    isPending && signInId ? (
      <SignInActions
        device={device}
        busy={loadingAction !== null}
        onItWasMe={() => void act(notification.id, 'approve')}
        onSignOut={async () => {
          await signOutDevice(signInId)
          toastAlert(t('sign_out_device_done', { device: device ?? t('new_signin_unknown') }))
          onAnswered(notification.id, 'decline')
        }}
      />
    ) : null

  const quoted = !ranOut && !stuckOpen && !dialog && !signInActions && approvable
  const answer = ranOut || stuckOpen ? null : dialog ? (
    <DialogAnswer notification={notification} dialog={dialog} onAnswered={onAnswered} />
  ) : signInActions ? (
    signInActions
  ) : approvable ? (
    <ApprovalCard
      quoteTitle={t('approval_quote_inbox')}
      tool={notification.metadata.approval?.tool}
      args={notification.metadata.approval?.args}
      inWords={notification.description}
      loses={notification.metadata.approval?.loses}
      verb={notification.metadata.approval?.verb}
      busy={loadingAction !== null}
      error={failure}
      onDecline={() => void act(notification.id, 'reject')}
      onAllow={() => void act(notification.id, 'approve')}
    />
  ) : (
    <div className="a-card-actions">
      <button type="button" className="a-btn secondary" disabled={loadingAction !== null} onClick={() => void act(notification.id, 'dismiss')}>
        {t('inbox_clear')}
      </button>
    </div>
  )

  return (
    <article className="a-card needs" aria-current={selected ? 'true' : undefined}>
      <div className="a-card-head">
        <span className="a-av a-warn-t">
          <Icon className="a-ic" aria-hidden="true" />
        </span>
        <span className="hq-who">
          <b>
            <Link to="/inbox/$id" params={{ id: notification.id }} aria-label={t('inbox_open_title', { title: notification.title })}>
              {signInId ? t('new_signin_title') : notification.title}
            </Link>
          </b>
          <small className="a-cap a-muted">{where}</small>
        </span>
      </div>
      {signInId && <SignInBody device={device} createdAt={notification.created_at} />}
      {!signInId && !quoted && !dialog && <p className="a-body-sm">{notification.description}</p>}
      {failure && !approvable && (
        <p role="alert" className="a-stale">
          <TriangleAlert className="a-ic" aria-hidden="true" />
          <span>{failure}</span>
        </p>
      )}
      {refused && (
        <Link to="/helpers/$id" params={{ id: helperId }} className="a-btn secondary">
          {t('inbox_open_helper')}
        </Link>
      )}
      {sent && (
        <p role="status" className="a-cap a-muted">
          {t('inbox_answer_sent_at', { answer: sent.answer, time: clockTime(sent.at) })}
        </p>
      )}
      {ranOut && <p className="a-cap a-muted">{t('inbox_request_ran_out')}</p>}
      {stuckOpen && (
        <div className="a-card-actions">
          <Link to="/helpers/$id" params={{ id: helperId }} className="a-btn">
            {t('inbox_open_it_to_see')}
          </Link>
        </div>
      )}
      {answer}
    </article>
  )
}
