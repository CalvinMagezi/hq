import { Link } from '@tanstack/react-router'
import { ArrowLeft, ArrowUpRight, BellOff, Inbox } from 'lucide-react'
import { useState } from 'react'
import { PaneHead } from '~/components/ui/Panes'
import { allowLabel, allowTone, quoteArgs } from '~/components/ui/ApprovalCard'
import { SandboxGrantActions } from '~/components/ui/SandboxGrantCard'
import { answerErrorLine, lapsed, REQUEST_LIFETIME_MS, serverHqLine } from '~/lib/inboxCard'
import { silenceTopicOf, type NotificationActionOpts, type NotificationItem, type SilenceTopic } from '~/lib/notificationsApi'
import { ReminderActions } from '~/components/reminders/ReminderActions'
import {
  SANDBOX_GRANTS_QUERY_KEY,
  SANDBOX_STARTERS_QUERY_KEY,
  capabilityModeKey,
  isSandboxGrantNotification,
  listSandboxGrants,
  listSandboxStarters,
  parseSandboxGrantNotification,
} from '~/lib/sandboxGrantsApi'
import { t, type StringKey } from '~/strings'
import { useQuery } from '@tanstack/react-query'
import { listDevices, signOutDevice } from '~/lib/deviceSignIn'
import { toastAlert } from '~/lib/toast'
import { DialogAnswer, openDialog } from './DialogAnswer'
import { SignInActions, SignInBody } from './SignInItem'

const WHEN_FORMAT: Intl.DateTimeFormatOptions = { dateStyle: 'medium', timeStyle: 'short' }

interface Props {
  notification: NotificationItem | null
  onAction(id: string, action: string, opts?: NotificationActionOpts): Promise<void>
  onAnswered: (id: string, choice: 'approve' | 'decline') => void
  loadingAction: string | null
  /** Where the link in the item's own words goes. */
  onOpenLink?: (link: string) => void
  /** Silences the item's topic; what HQ asks itself has none and never offers it. */
  onSilence?: (topic: SilenceTopic) => void
  /** Brings an update back later; an approval has no Later. */
  onLater?: (id: string) => void
}

const SILENCE_LABEL: Record<SilenceTopic, StringKey> = {
  helpers: 'silence_this_helpers',
  tasks: 'silence_this_tasks',
  tips: 'silence_this_tips',
  upkeep: 'silence_this_upkeep',
  email: 'silence_this_email',
}

const formatWhen = (ms: number) => new Intl.DateTimeFormat(undefined, WHEN_FORMAT).format(ms)

/** The open item: its words, the quote of what it would run, the details, and the answers in a footer that stays put. */
export function NotificationDetail({ notification, onAction, onAnswered, loadingAction, onOpenLink, onSilence, onLater }: Props) {
  const [failure, setFailure] = useState<string | null>(null)
  const signInId = notification?.metadata.sign_in?.device_id
  const isSandboxGrant = notification ? isSandboxGrantNotification(notification) : false
  const devices = useQuery({ queryKey: ['auth-devices'], queryFn: listDevices, enabled: !!signInId && notification?.state === 'pending' })
  const sandboxGrants = useQuery({
    queryKey: SANDBOX_GRANTS_QUERY_KEY,
    queryFn: () => listSandboxGrants(),
    enabled: isSandboxGrant && notification?.state === 'pending',
    retry: false,
  })
  const sandboxStarters = useQuery({
    queryKey: SANDBOX_STARTERS_QUERY_KEY,
    queryFn: listSandboxStarters,
    enabled: isSandboxGrant && notification?.state === 'pending',
    retry: false,
  })
  const device = devices.data?.devices.find((d) => d.id === signInId)?.label ?? null
  const back = (
    <Link to="/inbox" className="a-ib a-back" aria-label={t('inbox_back')}>
      <ArrowLeft className="a-ic" aria-hidden="true" />
    </Link>
  )
  if (!notification) {
    return (
      <>
        <PaneHead title={t('inbox_details')} back={back} />
        <div className="a-empty">
          <span className="a-empty-ic">
            <Inbox className="a-ic" aria-hidden="true" />
          </span>
          <p>{t('inbox_d_pick')}</p>
        </div>
      </>
    )
  }
  const n = notification
  const dialog = openDialog(n)
  const isPending = n.state === 'pending'
  const approval = n.metadata.approval
  const parsedGrant = isSandboxGrant ? parseSandboxGrantNotification(n, sandboxGrants.data?.grants) : null
  const approvable = n.actions.includes('approve')
  const isRequest = !!approval || !!n.metadata.dialog || isSandboxGrant
  const made = new Date(n.created_at).getTime()
  const server = serverHqLine(n)
  const link = n.metadata.link
  const topic = silenceTopicOf(n)
  const answering = (action: string) => loadingAction === `${action}-${n.id}`
  const busy = loadingAction !== null
  // What the footer offers: a dialog and a sign-in have their own answers; a plain request is allowed or declined; an update is cleared.
  const plainBody = signInId ? <SignInBody device={device} createdAt={n.created_at} /> : <p className="hq-inbox-text">{n.description}</p>
  const askable = !dialog && !signInId && !isSandboxGrant && approvable
  const clearable = !dialog && !signInId && !isSandboxGrant && !approvable
  // A reminder's ping carries `snooze`: only HQ's own reminders do.
  const isReminder = clearable && n.actions.includes('snooze')
  const act = async (action: string, opts?: NotificationActionOpts) => {
    setFailure(null)
    try {
      await onAction(n.id, action, opts)
    } catch (e) {
      setFailure(answerErrorLine(e))
    }
  }

  const detailBody = parsedGrant ? (
    <>
      <blockquote className="a-quote" data-testid="grant-resource">
        <span className="a-cap a-muted">{t(capabilityModeKey(parsedGrant.capability))}</span>{' '}
        <code className="a-mono">{parsedGrant.resource}</code>
      </blockquote>
      {parsedGrant.taskId && (
        <div className="a-drow">
          <span>{t('grants_task_label')}</span>
          <span className="a-mono">{parsedGrant.taskId}</span>
        </div>
      )}
      {parsedGrant.repo && (
        <div className="a-drow">
          <span>{t('grants_repo_label')}</span>
          <span className="a-mono">{parsedGrant.repo}</span>
        </div>
      )}
      <p className="a-cap a-muted">{t('grants_expires_day')}</p>
      {parsedGrant.helperReason && (
        <div className="flex flex-col gap-1" data-testid="grant-helper-reason">
          <p className="a-cap a-muted">{t('grants_helper_words')}</p>
          <blockquote className="a-quote a-muted">{parsedGrant.helperReason}</blockquote>
        </div>
      )}
    </>
  ) : null

  return (
    <>
      <PaneHead title={t('inbox_details')} back={back} />
      <div className="hq-inbox-body">
        <div className="a-wrap hq-inbox-wrap">
          {server && <p className="a-body-sm a-muted">{server}</p>}
          <p className="a-h2">{n.title}</p>
          {detailBody ??
            (approval ? (
              <>
                <p className="a-label">{t('approval_quote_inbox')}</p>
                <pre className="a-quote">
                  {approval.tool}
                  {'\n'}
                  {quoteArgs(approval.args)}
                </pre>
                <p className="hq-inbox-text">
                  <i>{t('approval_in_words')}:</i> {n.description}
                </p>
              </>
            ) : (
              plainBody
            ))}
          {lapsed(n) && <p className="a-cap a-muted">{t('inbox_request_ran_out')}</p>}
          {link && onOpenLink && (
            <button type="button" className="a-btn secondary" onClick={() => onOpenLink(link)}>
              <ArrowUpRight className="a-ic sm" aria-hidden="true" />
              {t('inbox_d_open')}
            </button>
          )}
          <div>
            <div className="a-drow">
              <span>{t('inbox_d_asked_by')}</span>
              <span>{n.metadata.dialog ? t('inbox_d_helper') : t('approval_who_hq')}</span>
            </div>
            <div className="a-drow">
              <span>{t('inbox_d_when')}</span>
              <span>{Number.isFinite(made) ? formatWhen(made) : t('time_unknown')}</span>
            </div>
            {isPending && isRequest && Number.isFinite(made) && (
              <div className="a-drow">
                <span>{t('inbox_d_waits_until')}</span>
                <span>{formatWhen(made + REQUEST_LIFETIME_MS)}</span>
              </div>
            )}
            {n.metadata.score !== undefined && (
              <div className="a-drow">
                <span>{t('inbox_d_score')}</span>
                <span>{n.metadata.score.toFixed(2)}</span>
              </div>
            )}
          </div>
          {isPending && topic && n.silenced && (
            <p className="a-cap a-muted">
              <BellOff className="a-ic sm" aria-hidden="true" /> {t('inbox_silenced')}
            </p>
          )}
          {isPending && topic && !n.silenced && onSilence && (
            <button type="button" className="a-btn quiet compact" onClick={() => onSilence(topic)}>
              <BellOff className="a-ic sm" aria-hidden="true" />
              {t(SILENCE_LABEL[topic])}
            </button>
          )}
        </div>
      </div>
      {isPending && !lapsed(n) && (
        <div className="hq-inbox-foot">
          {failure && <p role="alert" className="a-cap a-muted">{failure}</p>}
          {dialog && <DialogAnswer notification={n} dialog={dialog} onAnswered={onAnswered} />}
          {signInId && (
            <SignInActions
              device={device}
              busy={busy}
              onItWasMe={() => void act('approve')}
              onSignOut={async () => {
                await signOutDevice(signInId)
                toastAlert(t('sign_out_device_done', { device: device ?? t('new_signin_unknown') }))
                onAnswered(n.id, 'decline')
              }}
            />
          )}
          {isSandboxGrant && parsedGrant && (
            <div className="w-full">
              <SandboxGrantActions
                grant={parsedGrant}
                starters={sandboxStarters.data?.starters}
                busy={busy}
                onAnswer={(ans) =>
                  void act(ans.action, ans.action === 'allow_starter' ? { starter: ans.starter } : undefined)
                }
              />
            </div>
          )}
          {askable && (
            <>
              <button type="button" className="a-btn secondary" disabled={busy} onClick={() => void act('reject')}>
                {t('notice_action_decline')}
              </button>
              <button
                type="button"
                className={allowTone(approval?.loses) === 'danger' ? 'a-btn danger' : 'a-btn secondary'}
                disabled={busy}
                aria-busy={answering('approve') || undefined}
                onClick={() => void act('approve')}
              >
                {answering('approve') ? t('approval_answering') : allowLabel(approval?.loses, approval?.verb)}
              </button>
            </>
          )}
          {isReminder && (
            <ReminderActions
              title={n.title}
              busy={busy}
              onAct={(action, until) => void act(action, until ? { until } : undefined)}
            />
          )}
          {clearable && (
            <button type="button" className="a-btn secondary" disabled={busy} onClick={() => void act('dismiss')}>
              {t('inbox_clear')}
            </button>
          )}
          {clearable && onLater && (
            <button type="button" className="a-btn secondary" disabled={busy} onClick={() => onLater(n.id)}>
              {t('inbox_later')}
            </button>
          )}
        </div>
      )}
    </>
  )
}
