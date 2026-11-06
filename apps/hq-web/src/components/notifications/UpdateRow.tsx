import { Link } from '@tanstack/react-router'
import { Bot, Check, ClipboardCheck, X } from 'lucide-react'
import type { NotificationItem } from '~/lib/notificationsApi'
import { serverHqLine } from '~/lib/inboxCard'
import { t } from '~/strings'
import { updateKind, type UpdateKind } from './updateKind'

const KIND_ICON = { tasks: ClipboardCheck, helpers: Bot, hq: Check } as const satisfies Record<UpdateKind, unknown>

/** The clock time of an item, in this browser's clock. */
export function clockTime(at: string | Date): string {
  return new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' }).format(new Date(at))
}

interface Props {
  notification: NotificationItem
  selected?: boolean
  /** Clears the update; omitted for an item that is already done. */
  onClear?: () => void
  busy?: boolean
}

/** One update or one answered item: an icon, its words, when, and Clear for a waiting update. */
export function UpdateRow({ notification: n, selected, onClear, busy }: Props) {
  const Icon = KIND_ICON[updateKind(n)]
  const from = serverHqLine(n)
  return (
    <div className="a-msg" data-testid="inbox-update">
      <span className="a-av">
        <Icon className="a-ic" aria-hidden="true" />
      </span>
      <Link
        to="/inbox/$id"
        params={{ id: n.id }}
        className="a-msg-main"
        aria-current={selected ? 'true' : undefined}
        aria-label={t('inbox_open_title', { title: n.title })}
      >
        <b>{n.title}</b>
        <span>{from ? `${from}. ${n.description}` : n.description}</span>
      </Link>
      <span className="a-msg-time">{clockTime(n.created_at)}</span>
      {onClear && (
        <button type="button" className="a-ib sm" disabled={busy} aria-label={t('inbox_clear_one', { title: n.title })} onClick={onClear}>
          <X className="a-ic sm" aria-hidden="true" />
        </button>
      )}
    </div>
  )
}
