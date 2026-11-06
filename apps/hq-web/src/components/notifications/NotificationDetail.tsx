import { Link } from '@tanstack/react-router'
import { ArrowLeft, ArrowUpRight, Inbox } from 'lucide-react'
import { useState } from 'react'
import { PaneHead } from '~/components/ui/Panes'
import { allowLabel, allowTone, quoteArgs } from '~/components/ui/ApprovalCard'
import { answerErrorLine, lapsed, REQUEST_LIFETIME_MS, serverHqLine } from '~/lib/inboxCard'
import type { NotificationItem } from '~/lib/notificationsApi'
import { t } from '~/strings'
import { DialogAnswer, openDialog } from './DialogAnswer'

const WHEN_FORMAT: Intl.DateTimeFormatOptions = { dateStyle: 'medium', timeStyle: 'short' }

interface Props {
  notification: NotificationItem | null
  onAction(id: string, action: string): Promise<void>
  onAnswered: (id: string, choice: 'approve' | 'decline') => void
  loadingAction: string | null
  /** Where the link in the item's own words goes. */
  onOpenLink?: (link: string) => void
}

const formatWhen = (ms: number) => new Intl.DateTimeFormat(undefined, WHEN_FORMAT).format(ms)

/** The open item: its words, the quote of what it would run, the details, and the answers in a footer that stays put. */
export function NotificationDetail({ notification, onAction, onAnswered, loadingAction, onOpenLink }: Props) {
  const [failure, setFailure] = useState<string | null>(null)
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
  const approvable = n.actions.includes('approve')
  const isRequest = !!approval || !!n.metadata.dialog
  const made = new Date(n.created_at).getTime()
  const server = serverHqLine(n)
  const link = n.metadata.link
  const answering = (action: string) => loadingAction === `${action}-${n.id}`
  const busy = loadingAction !== null
  const act = async (action: string) => {
    setFailure(null)
    try {
      await onAction(n.id, action)
    } catch (e) {
      setFailure(answerErrorLine(e))
    }
  }

  return (
    <>
      <PaneHead title={t('inbox_details')} back={back} />
      <div className="hq-inbox-body">
        <div className="a-wrap hq-inbox-wrap">
          {server && <p className="a-body-sm a-muted">{server}</p>}
          <p className="a-h2">{n.title}</p>
          {approval ? (
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
            <p className="hq-inbox-text">{n.description}</p>
          )}
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
        </div>
      </div>
      {isPending && !lapsed(n) && (
        <div className="hq-inbox-foot">
          {failure && <p role="alert" className="a-cap a-muted">{failure}</p>}
          {dialog && <DialogAnswer notification={n} dialog={dialog} onAnswered={onAnswered} />}
          {!dialog && approvable && (
            <>
              <button type="button" className="a-btn secondary" disabled={busy} onClick={() => void act('reject')}>
                {t('notice_action_decline')}
              </button>
              <button
                type="button"
                className={allowTone(approval?.loses) === 'danger' ? 'a-btn danger' : 'a-btn'}
                disabled={busy}
                aria-busy={answering('approve') || undefined}
                onClick={() => void act('approve')}
              >
                {answering('approve') ? t('approval_answering') : allowLabel(approval?.loses, approval?.verb)}
              </button>
            </>
          )}
          {!dialog && !approvable && (
            <button type="button" className="a-btn secondary" disabled={busy} onClick={() => void act('dismiss')}>
              {t('inbox_clear')}
            </button>
          )}
        </div>
      )}
    </>
  )
}
