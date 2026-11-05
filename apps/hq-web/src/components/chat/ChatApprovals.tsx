import { useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ApprovalCard } from '~/components/ui/ApprovalCard'
import { HqHttpError } from '~/lib/hqAuth'
import { approvalView, approvalsForThread, splitSaid } from '~/lib/chatApprovals'
import { executeNotificationAction, fetchNotificationsClient, type NotificationItem } from '~/lib/notificationsApi'
import { toastAlert } from '~/lib/toast'
import { t } from '~/strings'

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
  const [answeredHere, setAnsweredHere] = useState<Map<string, number>>(new Map())
  const [elsewhere, setElsewhere] = useState<Set<string>>(new Set())
  const seenPending = useRef<Set<string>>(new Set())

  const items = approvalsForThread(list.data ?? [], threadId)
  for (const i of items) if (i.state === 'pending') seenPending.current.add(i.id)
  const shown = items.filter((i) => i.state === 'pending' || seenPending.current.has(i.id))
  const anyPending = shown.some((i) => i.state === 'pending')

  const answer = async (item: NotificationItem, action: 'approve' | 'reject') => {
    setBusy(item.id)
    try {
      await executeNotificationAction(item.id, action)
      setAnsweredHere((m) => new Map(m).set(item.id, Date.now()))
    } catch (err) {
      if (err instanceof HqHttpError && err.status === HTTP_CONFLICT) setElsewhere((s) => new Set(s).add(item.id))
      else toastAlert(err instanceof Error ? err.message : t('notice_action_try_again'))
    } finally {
      setBusy(null)
      await client.invalidateQueries({ queryKey: key })
    }
  }

  if (!threadId || (shown.length === 0 && !waiting)) return null
  return (
    <div className="shrink-0 max-h-[45dvh] overflow-y-auto overscroll-contain px-3 sm:px-6 pb-2">
      <div className="max-w-4xl mx-auto space-y-2">
        {shown.map((item) => (
          <ApprovalEntry
            key={item.id}
            item={item}
            view={elsewhere.has(item.id) ? 'answered_elsewhere' : approvalView(item, new Set(answeredHere.keys()))}
            answeredAt={answeredHere.get(item.id)}
            busy={busy === item.id}
            waiting={waiting}
            onAnswer={(a) => void answer(item, a)}
            onKeepGoing={onKeepGoing}
          />
        ))}
        {waiting && !anyPending && shown.length === 0 && (
          <section className="a-card" aria-label={t('chat_approval_head')}>
            <p className="text-xs font-semibold">{t('chat_approval_head')}</p>
            <p className="text-xs" style={{ color: 'var(--warn)' }}>{t('chat_approval_state')}</p>
          </section>
        )}
      </div>
    </div>
  )
}

interface EntryProps {
  item: NotificationItem
  view: ReturnType<typeof approvalView>
  answeredAt?: number
  busy: boolean
  waiting: boolean
  onAnswer: (action: 'approve' | 'reject') => void
  onKeepGoing: () => void
}

function ApprovalEntry({ item, view, answeredAt, busy, waiting, onAnswer, onKeepGoing }: EntryProps) {
  const { said, rest } = splitSaid(item.description)
  const words = said?.replace(/^You said:\s*/, '').replace(/^[“"]|[”"]$/g, '') ?? null
  const quote = said ? (
    <p className="text-xs" data-testid="approval-said">
      {t('chat_approval_you_said', { words: words ?? '' })} · {clock(Date.parse(item.created_at))}
    </p>
  ) : null

  if (view === 'pending') {
    return (
      <div>
        <p className="text-xs font-semibold px-1 pb-1">
          {t('chat_approval_head')} · <span style={{ color: 'var(--warn)' }}>{t('chat_approval_state')}</span>
        </p>
        {quote}
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
        />
      </div>
    )
  }

  const stateLine =
    view === 'answered_elsewhere'
      ? t('chat_approval_elsewhere')
      : item.state === 'approved'
        ? t('chat_approval_allowed_at', { time: clock(answeredAt ?? Date.now()) })
        : t('chat_approval_answered')
  return (
    <section className="a-card" aria-label={t('chat_approval_head')}>
      <p className="text-xs font-semibold">
        {view === 'answered_elsewhere' ? stateLine : `${stateLine} · ${t('chat_approval_answered')}`}
      </p>
      {quote}
      {item.metadata.approval?.tool && <code className="text-xs">{item.metadata.approval.tool}</code>}
      {waiting && (
        <>
          <p className="text-xs">{t('chat_approval_stopped')}</p>
          <div className="a-dlg-actions">
            <button type="button" className="a-btn" onClick={onKeepGoing}>
              {t('chat_keep_going')}
            </button>
          </div>
        </>
      )}
    </section>
  )
}
