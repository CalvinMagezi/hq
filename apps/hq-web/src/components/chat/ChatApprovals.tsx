import { useRef, useState, type ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { ApprovalCard } from '~/components/ui/ApprovalCard'
import { SandboxGrantCard } from '~/components/ui/SandboxGrantCard'
import { HqHttpError } from '~/lib/hqAuth'
import { approvalsForThread, splitSaid } from '~/lib/chatApprovals'
import { executeNotificationAction, fetchNotificationsClient, type NotificationItem } from '~/lib/notificationsApi'
import {
  SANDBOX_GRANTS_QUERY_KEY,
  SANDBOX_RULES_QUERY_KEY,
  SANDBOX_STARTERS_QUERY_KEY,
  SANDBOX_SUMMARY_QUERY_KEY,
  isSandboxGrantNotification,
  listSandboxGrants,
  listSandboxStarters,
  parseSandboxGrantNotification,
  type SandboxGrant,
  type SandboxGrantProfile,
} from '~/lib/sandboxGrantsApi'
import { toastAlert } from '~/lib/toast'
import { t, tn } from '~/strings'

type ApprovalAction =
  | 'approve'
  | 'allow_task'
  | 'allow_repo'
  | 'allow_starter'
  | 'approve_site'
  | 'approve_folder'
  | 'reject'
  | 'dismiss'

const POLL_MS = 4000
const HTTP_CONFLICT = 409
const clock = (at: number) => new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })

interface Props {
  threadId: string | null
  /** Sessions this chat watches, so their sandbox-grants requests appear here too. */
  sessionIds?: string[]
  /** The last turn ended while HQ waits for the owner's yes. */
  waiting: boolean
  /** Asks HQ to finish in this chat once the request was answered after it stopped waiting. */
  onKeepGoing: () => void
  /** What the slot holds when no approval wants it: an approval always wins the one slot above the box. */
  otherwise?: ReactNode
}

/** The owner approvals of this chat: the same items as the Inbox, so answering in either answers both. */
export function ChatApprovals({ threadId, sessionIds = [], waiting, onKeepGoing, otherwise }: Props) {
  const client = useQueryClient()
  const key = ['chat-approvals', threadId]
  const list = useQuery({
    queryKey: key,
    queryFn: async () => (await fetchNotificationsClient({ state: 'all', limit: 100 })).notifications,
    enabled: !!threadId,
    refetchInterval: POLL_MS,
  })
  const [busy, setBusy] = useState<string | null>(null)
  const [errors, setErrors] = useState<Record<string, string | null>>({})
  const seenPending = useRef<Set<string>>(new Set())

  const items = approvalsForThread(list.data ?? [], threadId, sessionIds)
  for (const i of items) if (i.state === 'pending') seenPending.current.add(i.id)
  const pending = items.filter((i) => i.state === 'pending')
  const hasSandboxGrant = pending.some(isSandboxGrantNotification)
  const grants = useQuery({
    queryKey: SANDBOX_GRANTS_QUERY_KEY,
    queryFn: () => listSandboxGrants(),
    enabled: hasSandboxGrant,
    retry: false,
  })
  const starters = useQuery({
    queryKey: SANDBOX_STARTERS_QUERY_KEY,
    queryFn: listSandboxStarters,
    enabled: hasSandboxGrant,
    retry: false,
  })
  const answered = items.filter((i) => i.state !== 'pending' && seenPending.current.has(i.id)).length
  const canKeepGoing = pending.length === 0 && waiting && answered !== 0
  const showSummary = answered !== 0 && !waiting

  const answer = async (item: NotificationItem, action: ApprovalAction, opts?: { starter?: string }) => {
    setBusy(item.id)
    setErrors((prev) => ({ ...prev, [item.id]: null }))
    try {
      await executeNotificationAction(item.id, action, opts)
    } catch (err) {
      if (!(err instanceof HqHttpError && err.status === HTTP_CONFLICT)) {
        const msg = err instanceof Error ? err.message : t('notice_action_try_again')
        setErrors((prev) => ({ ...prev, [item.id]: msg }))
        toastAlert(msg)
      }
    } finally {
      setBusy(null)
      await client.invalidateQueries({ queryKey: key })
      if (isSandboxGrantNotification(item)) {
        await client.invalidateQueries({ queryKey: SANDBOX_GRANTS_QUERY_KEY })
        await client.invalidateQueries({ queryKey: SANDBOX_RULES_QUERY_KEY })
        await client.invalidateQueries({ queryKey: SANDBOX_SUMMARY_QUERY_KEY })
      }
    }
  }

  if (!threadId || (pending.length === 0 && answered === 0 && !waiting)) {
    return otherwise ? <div className="hq-slot">{otherwise}</div> : null
  }
  return (
    <div className="shrink-0 max-h-[45dvh] overflow-y-auto overscroll-contain px-3 sm:px-6 pb-2">
      <div className="max-w-4xl mx-auto space-y-2">
        {pending.map((item) => (
          <ApprovalEntry
            key={item.id}
            item={item}
            grants={grants.data?.grants}
            starters={starters.data?.starters}
            busy={busy === item.id}
            error={errors[item.id] ?? null}
            onAnswer={(a, opts) => void answer(item, a, opts)}
          />
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
  grants?: SandboxGrant[]
  starters?: SandboxGrantProfile[]
  busy: boolean
  error?: string | null
  onAnswer: (action: ApprovalAction, opts?: { starter?: string }) => void
}

function ApprovalEntry({ item, grants, starters, busy, error, onAnswer }: EntryProps) {
  if (isSandboxGrantNotification(item)) {
    const parsed = parseSandboxGrantNotification(item, grants)
    return (
      <div>
        <p className="text-xs font-semibold px-1 pb-1">
          {t('chat_approval_head')} · <span style={{ color: 'var(--warn)' }}>{t('chat_approval_state')}</span>
        </p>
        <SandboxGrantCard
          grant={parsed}
          starters={starters}
          busy={busy}
          error={error}
          onAnswer={(ans) =>
            onAnswer(ans.action, ans.action === 'allow_starter' ? { starter: ans.starter } : undefined)
          }
        />
      </div>
    )
  }
  const { said, rest } = splitSaid(item.description)
  const words = said?.replace(/^You said:\s*/, '').replace(/^[“"]|[”"]$/g, '') ?? null
  const site = item.metadata.approval?.site
  const folder = item.metadata.approval?.folder
  const requests = item.metadata.approval?.recent_requests ?? 0
  const nudge = requests >= (item.metadata.approval?.nudge_at ?? Number.POSITIVE_INFINITY)
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
          <>
            {site && (
              <button type="button" className="a-btn secondary" disabled={busy} onClick={() => onAnswer('approve_site')}>
                {t('approval_allow_site', { site })}
              </button>
            )}
            {folder && (
              <button
                type="button"
                className="a-btn secondary"
                disabled={busy}
                title={t('approval_folder_note')}
                onClick={() => onAnswer('approve_folder')}
              >
                {t('approval_allow_folder', { folder })}
              </button>
            )}
          </>
        }
      />
      {nudge && (
        <p className="text-xs a-muted px-1 pt-1" data-testid="approval-nudge">
          {t('approval_nudge', { n: requests })}{' '}
          <a href="/helpers" className="a-link">
            {t('approval_nudge_helpers')}
          </a>
        </p>
      )}
    </div>
  )
}
