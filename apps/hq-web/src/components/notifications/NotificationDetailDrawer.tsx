import { X, CheckCircle2, XCircle, FileCode, Check, Loader2, AlertCircle } from 'lucide-react'
import type { NotificationItem } from '~/lib/notificationsApi'

interface Props {
  notification: NotificationItem | null
  onClose: () => void
  onAction: (id: string, action: string) => Promise<void>
  loadingAction: string | null
}

export function NotificationDetailDrawer({
  notification,
  onClose,
  onAction,
  loadingAction,
}: Props) {
  if (!notification) return null

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
    <div className="fixed inset-0 z-50 flex justify-end bg-black/60 backdrop-blur-sm transition-all duration-300">
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
              Summary
            </h3>
            <p className="text-sm text-neutral-200 leading-relaxed whitespace-pre-wrap">
              {notification.description}
            </p>
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
              Item Details
            </h4>
            <div className="grid grid-cols-2 gap-3 text-xs ">
              <div>
                <span className="text-neutral-500 block">ID:</span>
                <span className="text-neutral-300 break-all">{notification.id}</span>
              </div>
              <div>
                <span className="text-neutral-500 block">Created:</span>
                <span className="text-neutral-300">
                  {new Date(notification.created_at).toLocaleString()}
                </span>
              </div>
              {notification.metadata.score !== undefined && (
                <div>
                  <span className="text-neutral-500 block">Score:</span>
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
            {notification.kind === 'action_needed' ? (
              <>
                <button
                  type="button"
                  onClick={handleDismiss}
                  disabled={loadingAction !== null}
                  className="px-4 py-2 rounded-xl text-xs font-semibold text-neutral-400 hover:text-neutral-200 hover:bg-white/5 transition-all disabled:opacity-50"
                >
                  {loadingAction === 'dismiss' ? <Loader2 className="w-4 h-4 animate-spin" /> : 'Dismiss'}
                </button>

                <button
                  type="button"
                  onClick={handleReject}
                  disabled={loadingAction !== null}
                  className="px-4 py-2 rounded-xl text-xs font-bold flex items-center gap-1.5 text-rose-400 hover:bg-rose-500/10 border border-rose-500/20 transition-all disabled:opacity-50"
                >
                  {loadingAction === 'reject' ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <>
                      <XCircle className="w-4 h-4" />
                      Reject
                    </>
                  )}
                </button>

                <button
                  type="button"
                  onClick={handleApprove}
                  disabled={loadingAction !== null}
                  className="px-5 py-2 rounded-xl text-xs font-bold flex items-center gap-1.5 transition-all disabled:opacity-50"
                  style={{
                    background: 'var(--accent-green, #00ffa3)',
                    color: '#000',
                  }}
                >
                  {loadingAction === 'approve' ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : (
                    <>
                      <Check className="w-4 h-4" />
                      Approve & Promote
                    </>
                  )}
                </button>
              </>
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
                    Dismiss
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
