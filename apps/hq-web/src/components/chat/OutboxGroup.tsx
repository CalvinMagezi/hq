import { t } from '~/strings'
import { lineFor, type OutboxEntry, type OutboxLineState } from '~/lib/outbox'

interface Props {
  entries: OutboxEntry[]
  state: OutboxLineState
  inHub: boolean
  /** "Sending {n} of {m}" while the drain runs; the entry it is on. */
  sending?: { n: number; m: number; client_id: string }
  textOf: (entry: OutboxEntry) => string
  onRemove: (clientId: string) => void
  onSendNow: (clientId: string) => void
}

export function OutboxGroup({ entries, state, inHub, sending, textOf, onRemove, onSendNow }: Props) {
  if (entries.length === 0) return null
  return (
    <section aria-label={t('outbox_heading')}>
      <h3 className="a-grp">
        {t('outbox_heading')} <em className="a-count">{entries.length}</em>
      </h3>
      {entries.map((e) => (
        <div className="a-li" key={e.client_id}>
          <div className="a-li-main">
            <b>{textOf(e)}</b>
            <span>
              {e.waited
                ? t('fail_send_message_waited')
                : sending?.client_id === e.client_id
                  ? t('outbox_sending', { n: sending.n, m: sending.m })
                  : lineFor(state, inHub)}
            </span>
          </div>
          <div className="a-li-trail">
            {e.waited && (
              <button type="button" onClick={() => onSendNow(e.client_id)}>
                {t('outbox_send_now')}
              </button>
            )}
            <button type="button" onClick={() => onRemove(e.client_id)}>
              {t('outbox_remove')}
            </button>
          </div>
        </div>
      ))}
    </section>
  )
}
