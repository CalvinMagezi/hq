import { ApprovalCard } from '~/components/ui/ApprovalCard'
import { SandboxGrantCard } from '~/components/ui/SandboxGrantCard'
import { t } from '~/strings'
import { useState, type ComponentType, type ReactNode } from 'react'
import { Link } from '@tanstack/react-router'
import { answerErrorLine, cardKind, lapsed, serverHqLine, taskCheckId, type CardKind } from '~/lib/inboxCard'
import { ClipboardCheck, LifeBuoy, ShieldAlert, Sparkles, TriangleAlert } from 'lucide-react'
import type { NotificationItem } from '~/lib/notificationsApi'
import {
  SANDBOX_GRANTS_QUERY_KEY,
  SANDBOX_STARTERS_QUERY_KEY,
  isSandboxGrantNotification,
  listSandboxGrants,
  listSandboxStarters,
  parseSandboxGrantNotification,
} from '~/lib/sandboxGrantsApi'
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
  onAction(id: string, action: string, opts?: { starter?: string }): Promise<void>
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
  const sandboxGrant = isSandboxGrantNotification(notification)
  const act = async (id: string, action: string, opts?: { starter?: string }) => {
    setFailure(null)
    setRefused(false)
    try {
      await onAction(id, action, opts)
      const isAllow = action === 'approve' || action === 'allow_task' || action === 'allow_repo' || action === 'allow_starter'
      if (isAllow || action === 'reject' || (sandboxGrant && action === 'dismiss')) {
        setSent({ answer: t(isAllow ? 'inbox_answer_allowed' : 'inbox_answer_declined'), at: new Date() })
      }
    } catch (e) {
      setFailure(answerErrorLine(e))
      setRefused((e as { status?: number } | null)?.status === 409)
    }
  }
  const dialog = openDialog(notification)
  const signInId = notification.metadata.sign_in?.device_id
  const devices = useQuery({ queryKey: ['auth-devices'], queryFn: listDevices, enabled: !!signInId && isPending })
  const sandboxGrants = useQuery({
    queryKey: SANDBOX_GRANTS_QUERY_KEY,
    queryFn: () => listSandboxGrants(),
    enabled: sandboxGrant && isPending,
    retry: false,
  })
  const sandboxStarters = useQuery({
    queryKey: SANDBOX_STARTERS_QUERY_KEY,
    queryFn: listSandboxStarters,
    enabled: sandboxGrant && isPending,
    retry: false,
  })
  const device = devices.data?.devices.find((d) => d.id === signInId)?.label ?? null
  const helperId = notification.metadata.extra?.session_id ?? notification.id
  const Icon = KIND_ICON[kind]
  const where = [serverHqLine(notification), relTime(notification.created_at)].filter(Boolean).join(' · ')
  // A task to check is looked over on its own page, never approved from the card.
  const taskCheck = kind === 'task_check'
  const approvable = !taskCheck && notification.actions.includes('approve')
  const checkTaskId = taskCheck ? taskCheckId(notification) : null

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

  const parsedGrant = sandboxGrant ? parseSandboxGrantNotification(notification, sandboxGrants.data?.grants) : null
  const quoted = !ranOut && !stuckOpen && !dialog && !signInActions && (sandboxGrant || approvable)
  let answer: ReactNode = null
  if (!ranOut && !stuckOpen) {
    if (dialog) {
      answer = <DialogAnswer notification={notification} dialog={dialog} onAnswered={onAnswered} />
    } else if (signInActions) {
      answer = signInActions
    } else if (sandboxGrant && parsedGrant && isPending) {
      answer = (
        <SandboxGrantCard
          grant={parsedGrant}
          starters={sandboxStarters.data?.starters}
          showTitle={false}
          busy={loadingAction !== null}
          error={failure}
          onAnswer={(ans) =>
            void act(
              notification.id,
              ans.action,
              ans.action === 'allow_starter' ? { starter: ans.starter } : undefined,
            )
          }
        />
      )
    } else if (taskCheck && isPending) {
      answer = (
        <div className="a-card-actions">
          <Link to="/tasks" search={checkTaskId ? { task: checkTaskId } : {}} className="a-btn">
            {t('tasks_check_it')}
          </Link>
        </div>
      )
    } else if (approvable) {
      answer = (
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
      )
    } else {
      answer = (
        <div className="a-card-actions">
          <button type="button" className="a-btn secondary" disabled={loadingAction !== null} onClick={() => void act(notification.id, 'dismiss')}>
            {t('inbox_clear')}
          </button>
        </div>
      )
    }
  }

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
      {!signInId && !quoted && !dialog && <p className="a-body-sm">{taskCheck ? t('tasks_finished_plain') : notification.description}</p>}
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
