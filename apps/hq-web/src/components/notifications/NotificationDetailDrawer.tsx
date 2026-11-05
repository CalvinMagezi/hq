import { ApprovalCard } from '~/components/ui/ApprovalCard'
import { t } from '~/strings'
import { useRouter } from '@tanstack/react-router'
import { X, CheckCircle2, XCircle, FileCode, Check, Loader2, AlertCircle, ArrowUpRight } from 'lucide-react'
import type { NotificationItem } from '~/lib/notificationsApi'
import { DialogAnswer, openDialog } from './DialogAnswer'

interface Props {
  notification: NotificationItem | null
  onClose: () => void
  onAction: (id: string, action: string) => Promise<void>
  onAnswered: (id: string, choice: 'approve' | 'decline') => void
  loadingAction: string | null
}

export function NotificationDetailDrawer({
  notification,
  onClose,
  onAction,
  onAnswered,
  loadingAction,
}: Props) {
  const router = useRouter()
  if (!notification) return null
  const dialog = openDialog(notification)

  const isPending = notification.state === 'pending'
  const isValue = notification.kind === 'value_item'

  const handleApprove = async () => {
    await onAction(notification.id, 'approve')
  }

  const handleReject = async () => {
    await onAction(notification.id, 'reject')
  }

  const handleDismiss = async () => {
    await onAction(notification.id, 'dismiss')
  }

  return (
    <div className="fixed inset-0 z-50 flex justify-end hq-scrim transition-all duration-300">
      {/* Backdrop click */}
      <div className="flex-1" onClick={onClose} />

      <div
        className="w-full max-w-2xl h-full flex flex-col glass-heavy border-l shadow-2xl overflow-hidden animate-in slide-in-from-right pad-safe-top pb-[var(--safe-bottom)]"
        style={{
          background: 'var(--bg-card, #111418)',
          borderColor: 'rgba(255,255,255,0.1)',
        }}
      >
        {/* Header */}
        <div
          className="flex items-center justify-between px-6 py-4 border-b flex-shrink-0"
          style={{ borderColor: 'rgba(255,255,255,0.08)' }}
        >
          <div className="flex items-center gap-2.5">
            {isValue && <FileCode className="w-5 h-5 text-amber-400" />}
            {!isValue && <AlertCircle className="w-5 h-5 text-purple-400" />}
            <div>
              <div className="flex items-center gap-2">
                <span className="text-[11px] font-bold uppercase tracking-wider px-2 py-0.5 rounded-full bg-white/5 text-neutral-300">
                  {notification.kind.replace('_', ' ')}
                </span>
                <span
                  className="text-[11px] font-semibold uppercase px-2 py-0.5 rounded-full"
                  style={{
                    background:
                      notification.state === 'pending'
                        ? 'rgba(0,255,163,0.15)'
                        : 'rgba(255,255,255,0.06)',
                    color:
                      notification.state === 'pending'
                        ? 'var(--accent-green, #00ffa3)'
                        : 'var(--text-dim, #888)',
                  }}
                >
                  {notification.state}
                </span>
              </div>
              <h2 className="text-base font-semibold text-white mt-1">{notification.title}</h2>
            </div>
          </div>

          <button
            type="button"
            onClick={onClose}
            className="p-1.5 rounded-lg text-neutral-400 hover:text-white hover:bg-white/10 transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Content Area */}
        <div className="flex-1 overflow-y-auto px-6 py-5 space-y-6">
          {/* Summary / Description */}
          <div>
            <h3 className="text-xs font-semibold text-neutral-400 uppercase tracking-wider mb-1.5">
              {t('inbox_d_summary')}
            </h3>
            <p className="text-sm text-neutral-200 leading-relaxed whitespace-pre-wrap">
              {notification.description}
            </p>
            {notification.metadata.link && (
              <button
                type="button"
                onClick={() => {
                  onClose()
                  router.history.push(notification.metadata.link!)
                }}
                className="hq-btn-ghost mt-3"
              >
                <ArrowUpRight className="w-3.5 h-3.5" />
                <span>{t('inbox_d_open')}</span>
              </button>
            )}
          </div>

          {/* Metadata details */}
          <div
            className="p-4 rounded-xl border space-y-2"
            style={{
              background: 'rgba(255,255,255,0.02)',
              borderColor: 'rgba(255,255,255,0.06)',
            }}
          >
            <h4 className="text-xs font-semibold text-neutral-400 uppercase tracking-wider">
              {t('inbox_d_details')}
            </h4>
            <div className="grid grid-cols-2 gap-3 text-xs ">
              <div>
                <span className="text-neutral-500 block">{t('inbox_d_id')}</span>
                <span className="text-neutral-300 break-all">{notification.id}</span>
              </div>
              <div>
                <span className="text-neutral-500 block">{t('inbox_d_created')}</span>
                <span className="text-neutral-300">
                  {new Date(notification.created_at).toLocaleString()}
                </span>
              </div>
              {notification.metadata.score !== undefined && (
                <div>
                  <span className="text-neutral-500 block">{t('inbox_d_score')}</span>
                  <span className="text-amber-400 font-bold">
                    {notification.metadata.score.toFixed(2)}
                  </span>
                </div>
              )}
            </div>
          </div>
        </div>

        {/* Footer Actions */}
        {isPending && (
          <div
            className="p-4 border-t flex items-center justify-end gap-3 flex-shrink-0"
            style={{
              borderColor: 'rgba(255,255,255,0.08)',
              background: 'rgba(0,0,0,0.4)',
            }}
          >
            {dialog ? (
              <DialogAnswer notification={notification} dialog={dialog} onAnswered={onAnswered} />
            ) : notification.kind === 'action_needed' ? (
              <ApprovalCard
                quoteTitle={t('approval_quote_inbox')}
                tool={notification.metadata.approval?.tool}
                args={notification.metadata.approval?.args}
                inWords={notification.description}
                loses={notification.metadata.approval?.loses}
                verb={notification.metadata.approval?.verb}
                busy={loadingAction !== null}
                onDecline={handleReject}
                onAllow={handleApprove}
              />
            ) : (
              <button
                type="button"
                onClick={handleDismiss}
                disabled={loadingAction !== null}
                className="px-5 py-2 rounded-xl text-xs font-semibold text-neutral-200 hover:text-white hover:bg-white/10 border border-white/10 transition-all flex items-center gap-1.5 disabled:opacity-50"
              >
                {loadingAction === 'dismiss' ? (
                  <Loader2 className="w-4 h-4 animate-spin" />
                ) : (
                  <>
                    <Check className="w-4 h-4 text-emerald-400" />
                    {t('inbox_clear')}
                  </>
                )}
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
