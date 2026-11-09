import { useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ApprovalCard } from '~/components/ui/ApprovalCard'
import { HqHttpError } from '~/lib/hqAuth'
import { approvalsForThread, splitSaid } from '~/lib/chatApprovals'
import { executeNotificationAction, fetchNotificationsClient, type NotificationItem } from '~/lib/notificationsApi'
import { toastAlert } from '~/lib/toast'
import { t, tn } from '~/strings'

type ApprovalAction = 'approve' | 'approve_site' | 'reject'

const POLL_MS = 4000
const HTTP_CONFLICT = 409
const clock = (at: number) => new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })

interface Props {
  threadId: string | null
  /** The last turn ended while HQ waits for the owner's yes. */
  waiting: boolean
  /** Asks HQ to finish in this chat once the request was answered after it stopped waiting. */
  onKeepGoing: () => void
}

/** The owner approvals of this chat: the same items as the Inbox, so answering in either answers both. */
export function ChatApprovals({ threadId, waiting, onKeepGoing }: Props) {
  const client = useQueryClient()
  const key = ['chat-approvals', threadId]
  const list = useQuery({
    queryKey: key,
    queryFn: async () => (await fetchNotificationsClient({ state: 'all', limit: 100 })).notifications,
    enabled: !!threadId,
    refetchInterval: POLL_MS,
  })
  const [busy, setBusy] = useState<string | null>(null)
  const seenPending = useRef<Set<string>>(new Set())

  const items = approvalsForThread(list.data ?? [], threadId)
  for (const i of items) if (i.state === 'pending') seenPending.current.add(i.id)
  const pending = items.filter((i) => i.state === 'pending')
  const answered = items.filter((i) => i.state !== 'pending' && seenPending.current.has(i.id)).length
  const canKeepGoing = pending.length === 0 && waiting && answered !== 0
  const showSummary = answered !== 0 && !waiting

  const answer = async (item: NotificationItem, action: ApprovalAction) => {
    setBusy(item.id)
    try {
      await executeNotificationAction(item.id, action)
    } catch (err) {
      if (!(err instanceof HqHttpError && err.status === HTTP_CONFLICT)) {
        toastAlert(err instanceof Error ? err.message : t('notice_action_try_again'))
      }
    } finally {
      setBusy(null)
      await client.invalidateQueries({ queryKey: key })
    }
  }

  if (!threadId || (pending.length === 0 && answered === 0 && !waiting)) return null
  return (
    <div className="shrink-0 max-h-[45dvh] overflow-y-auto overscroll-contain px-3 sm:px-6 pb-2">
      <div className="max-w-4xl mx-auto space-y-2">
        {pending.map((item) => (
          <ApprovalEntry key={item.id} item={item} busy={busy === item.id} onAnswer={(a) => void answer(item, a)} />
        ))}
        {pending.length === 0 && waiting && answered === 0 && (
          <section className="a-card" aria-label={t('chat_approval_head')}>
            <p className="text-xs font-semibold">{t('chat_approval_head')}</p>
            <p className="text-xs" style={{ color: 'var(--warn)' }}>{t('chat_approval_state')}</p>
          </section>
        )}
        {canKeepGoing && (
          <section className="a-card" aria-label={t('chat_approval_head')}>
            <p className="text-xs">{t('chat_approval_stopped')}</p>
            <div className="a-dlg-actions">
              <button type="button" className="a-btn" onClick={onKeepGoing}>
                {t('chat_keep_going')}
              </button>
            </div>
          </section>
        )}
        {showSummary && <p className="text-xs a-muted px-1">{tn('chat_approval_summary', answered)}</p>}
      </div>
    </div>
  )
}

interface EntryProps {
  item: NotificationItem
  busy: boolean
  onAnswer: (action: ApprovalAction) => void
}

function ApprovalEntry({ item, busy, onAnswer }: EntryProps) {
  const { said, rest } = splitSaid(item.description)
  const words = said?.replace(/^You said:\s*/, '').replace(/^[“"]|[”"]$/g, '') ?? null
  const site = item.metadata.approval?.site
  return (
    <div>
      <p className="text-xs font-semibold px-1 pb-1">
        {t('chat_approval_head')} · <span style={{ color: 'var(--warn)' }}>{t('chat_approval_state')}</span>
      </p>
      {said && (
        <p className="text-xs" data-testid="approval-said">
          {t('chat_approval_you_said', { words: words ?? '' })} · {clock(Date.parse(item.created_at))}
        </p>
      )}
      <ApprovalCard
        quoteTitle={t('approval_quote_chat')}
        tool={item.metadata.approval?.tool}
        args={item.metadata.approval?.args}
        inWords={rest || item.description}
        loses={item.metadata.approval?.loses}
        verb={item.metadata.approval?.verb}
        busy={busy}
        onDecline={() => onAnswer('reject')}
        onAllow={() => onAnswer('approve')}
        more={
          site ? (
            <button type="button" className="a-btn secondary" disabled={busy} onClick={() => onAnswer('approve_site')}>
              {t('approval_allow_site', { site })}
            </button>
          ) : undefined
        }
      />
    </div>
  )
}
