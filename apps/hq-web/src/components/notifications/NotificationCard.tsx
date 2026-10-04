import React from 'react'
import {
  CheckCircle2,
  XCircle,
  Clock,
  ArrowRight,
  Loader2,
  FileCode,
  AlertCircle,
  Eye,
  Check,
  X,
} from 'lucide-react'
import type { NotificationItem, NotificationKind, NotificationState } from '~/lib/notificationsApi'
import { relTime } from '~/lib/time'

interface Props {
  notification: NotificationItem
  onSelect: (notification: NotificationItem) => void
  onAction: (id: string, action: string) => Promise<void>
  loadingAction: string | null
}

export function NotificationCard({
  notification,
  onSelect,
  onAction,
  loadingAction,
}: Props) {
  const isPending = notification.state === 'pending'

  const getKindIcon = (kind: NotificationKind) => {
    switch (kind) {
      case 'action_needed':
        return <FileCode className="w-4 h-4 text-cyan-400" />
      case 'value_item':
        return <AlertCircle className="w-4 h-4 text-amber-400" />
      default:
        return <AlertCircle className="w-4 h-4 text-purple-400" />
    }
  }

  const getKindBadgeClass = (kind: NotificationKind) => {
    switch (kind) {
      case 'action_needed':
        return 'bg-cyan-500/10 text-cyan-400 border-cyan-500/20'
      case 'value_item':
        return 'bg-amber-500/10 text-amber-400 border-amber-500/20'
      default:
        return 'bg-purple-500/10 text-purple-400 border-purple-500/20'
    }
  }

  return (
    <div
      className={`group relative min-w-0 rounded-2xl border p-4 sm:p-5 transition-all duration-200 ${
        isPending
          ? 'bg-black/30 hover:bg-black/40 border-white/10 hover:border-emerald-500/30'
          : 'bg-black/10 border-white/5 opacity-75 hover:opacity-100'
      }`}
      style={{
        backdropFilter: 'blur(12px)',
      }}
    >
      {/* Top row: Badges + Timestamp */}
      <div className="flex flex-wrap items-center justify-between gap-2 mb-2.5 min-w-0">
        <div className="flex flex-wrap items-center gap-2 min-w-0">
          <div className="p-1.5 rounded-lg bg-white/5 flex items-center justify-center">
            {getKindIcon(notification.kind)}
          </div>
          <span
            className={`text-[10px] font-mono font-bold uppercase tracking-wider px-2 py-0.5 rounded-full border ${getKindBadgeClass(
              notification.kind
            )}`}
          >
            {notification.kind.replace('_', ' ')}
          </span>
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <span className="text-[11px] font-mono text-neutral-500 flex items-center gap-1 whitespace-nowrap">
            <Clock className="w-3 h-3" />
            {relTime(notification.created_at)}
          </span>

          <span
            className="text-[10px] font-mono font-bold uppercase tracking-wider px-2 py-0.5 rounded-full"
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
      </div>

      {/* Title & Description */}
      <div className="mb-3.5">
        <h3 className="text-sm font-semibold text-white break-words group-hover:text-emerald-300 transition-colors">
          {notification.title}
        </h3>
        <p className="text-xs text-neutral-400 mt-1 line-clamp-2 leading-relaxed">
          {notification.description}
        </p>
      </div>

      {/* Action Footer */}
      <div className="flex flex-wrap items-center justify-between gap-2 pt-2 border-t border-white/5">
        <button
          type="button"
          onClick={() => onSelect(notification)}
          className="text-xs font-mono text-neutral-400 hover:text-emerald-400 flex items-center gap-1.5 transition-colors"
        >
          <Eye className="w-3.5 h-3.5" />
          <span>View Details & Diff</span>
        </button>

        {isPending ? (
          <div className="flex flex-wrap items-center justify-end gap-2 ml-auto">
            {notification.kind === 'action_needed' ? (
              <>
                <button
                  type="button"
                  onClick={() => onAction(notification.id, 'dismiss')}
                  disabled={loadingAction !== null}
                  className="p-1.5 rounded-lg text-neutral-400 hover:text-neutral-200 hover:bg-white/10 transition-colors disabled:opacity-50"
                  title="Dismiss"
                >
                  {loadingAction === `dismiss-${notification.id}` ? (
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  ) : (
                    <X className="w-3.5 h-3.5" />
                  )}
                </button>

                <button
                  type="button"
                  onClick={() => onAction(notification.id, 'reject')}
                  disabled={loadingAction !== null}
                  className="px-3 py-1.5 rounded-xl text-xs font-mono font-bold text-rose-400 hover:bg-rose-500/10 border border-rose-500/20 transition-all flex items-center gap-1 disabled:opacity-50"
                >
                  {loadingAction === `reject-${notification.id}` ? (
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  ) : (
                    <>
                      <XCircle className="w-3.5 h-3.5" />
                      Reject
                    </>
                  )}
                </button>

                <button
                  type="button"
                  onClick={() => onAction(notification.id, 'approve')}
                  disabled={loadingAction !== null}
                  className="px-3.5 py-1.5 rounded-xl text-xs font-mono font-bold flex items-center gap-1 transition-all disabled:opacity-50"
                  style={{
                    background: 'var(--accent-green, #00ffa3)',
                    color: '#000',
                  }}
                >
                  {loadingAction === `approve-${notification.id}` ? (
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  ) : (
                    <>
                      <Check className="w-3.5 h-3.5" />
                      Approve
                    </>
                  )}
                </button>
              </>
            ) : (
              <button
                type="button"
                onClick={() => onAction(notification.id, 'dismiss')}
                disabled={loadingAction !== null}
                className="px-3 py-1.5 rounded-xl text-xs font-mono font-semibold text-neutral-300 hover:text-white hover:bg-white/10 border border-white/10 transition-all flex items-center gap-1.5 disabled:opacity-50"
              >
                {loadingAction === `dismiss-${notification.id}` ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <>
                    <Check className="w-3.5 h-3.5 text-emerald-400" />
                    Dismiss
                  </>
                )}
              </button>
            )}
          </div>
        ) : (
          <div className="flex items-center gap-1.5 text-[11px] font-mono text-neutral-500">
            <CheckCircle2 className="w-3.5 h-3.5 text-neutral-400" />
            <span>Resolved</span>
          </div>
        )}
      </div>
    </div>
  )
}
