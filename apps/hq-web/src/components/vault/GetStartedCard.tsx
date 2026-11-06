import { useEffect, useState, useSyncExternalStore } from 'react'
import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { Check, Circle } from 'lucide-react'
import { t } from '~/strings'
import { CHAT_APPS_QUERY_KEY, fetchChatApps } from '~/lib/chatAppsApi'
import { needsSignIn, onSignInChange } from '~/lib/hqAuth'
import { getPushState, isStandalone, type PushState } from '~/lib/webPush'
import { usePersistedState } from '~/lib/usePersistedState'
import {
  DEFAULT_GET_STARTED_PREFS,
  GET_STARTED_KEY,
  getStartedItems,
  getStartedVisible,
  progressLine,
  isGetStartedPrefs,
  serverUnreachable,
  type GetStartedItem,
} from '~/lib/getStarted'

// A service worker that never becomes ready would otherwise keep the card from ever settling.
const PUSH_CHECK_TIMEOUT_MS = 5000

type PushCheck = { settled: false } | { settled: true; state: PushState | null }

function usePushCheck(enabled: boolean): PushCheck {
  const [check, setCheck] = useState<PushCheck>({ settled: false })
  useEffect(() => {
    if (!enabled) return
    let alive = true
    let timer = 0
    const timeout = new Promise<null>((resolve) => {
      timer = window.setTimeout(() => resolve(null), PUSH_CHECK_TIMEOUT_MS)
    })
    Promise.race([getPushState(), timeout])
      .catch(() => null)
      .then((state) => alive && setCheck({ settled: true, state }))
    return () => {
      alive = false
      window.clearTimeout(timer)
    }
  }, [enabled])
  return check
}

function ItemRow({ item, onMarkDone, phoneHidden }: { item: GetStartedItem; onMarkDone?: () => void; phoneHidden?: boolean }) {
  const Icon = item.done ? Check : Circle
  return (
    <li className={`${phoneHidden ? 'hidden sm:flex' : 'flex'} items-start gap-3 py-3 border-t first:border-t-0`} style={{ borderColor: 'var(--hairline)' }}>
      <Icon
        className="a-ic sm"
        aria-hidden="true"
      />
      <div className="flex flex-col gap-1 min-w-0 flex-1">
        <b className={item.done ? 'a-muted' : undefined}>
          {item.title}
          {item.done && <span className="a-body-sm a-muted"> {t('gs_done')}</span>}
        </b>
        {item.done && item.doneDetail && (
          <p className="a-body-sm a-muted">{item.doneDetail}</p>
        )}
        {!item.done && (
          <>
            <p className="a-body-sm a-muted">{item.detail}</p>
            <div className="flex flex-wrap items-center gap-2 pt-1">
              <Link
                to={item.to}
                hash={item.hash}
                className="a-btn secondary compact"
              >
                {item.linkLabel}
              </Link>
              {onMarkDone && (
                <button
                  type="button"
                  onClick={onMarkDone}
                  className="a-btn quiet compact"
                >
                  {t('gs_mark_done')}
                </button>
              )}
            </div>
          </>
        )}
      </div>
    </li>
  )
}

/** First-run checklist for the Chat start screen: the things an owner would not otherwise find. `compact` shows only the next step on a phone. */
export function GetStartedCard({ compact = false }: { compact?: boolean } = {}) {
  const [prefs, setPrefs] = usePersistedState(GET_STARTED_KEY, DEFAULT_GET_STARTED_PREFS, isGetStartedPrefs)
  const signInNeeded = useSyncExternalStore(onSignInChange, needsSignIn, () => false)
  const chatApps = useQuery({ queryKey: CHAT_APPS_QUERY_KEY, queryFn: fetchChatApps, enabled: !prefs.hidden })
  const push = usePushCheck(!prefs.hidden)

  const unreachable = chatApps.isError && serverUnreachable(chatApps.error)
  const items = getStartedItems({
    prefs,
    standalone: isStandalone(),
    chatApps: chatApps.data,
    push: push.settled ? push.state : null,
  })
  const settled = !chatApps.isPending && push.settled
  const allDone = settled && items.every((i) => i.done)

  // Once everything is done the card retires on this device, so later loads skip the checks.
  useEffect(() => {
    if (allDone && !prefs.hidden) setPrefs((p) => ({ ...p, hidden: true }))
  }, [allDone, prefs.hidden, setPrefs])

  if (!getStartedVisible({ settled, unavailable: signInNeeded || unreachable, prefs, items })) return null

  return (
    <section className="mb-10" aria-labelledby="get-started-title">
      <div className="flex items-center gap-2 mb-4">
        <h2 id="get-started-title" className="a-grp">
          {t('gs_title')}
        </h2>
        <span className="a-body-sm a-muted">{progressLine(items)}</span>
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => setPrefs((p) => ({ ...p, hidden: true }))}
          className="a-btn quiet compact"
        >
          {t('gs_hide')}
        </button>
      </div>
      <ul className="a-card">
        {items.map((item) => (
          <ItemRow
            key={item.id}
            item={item}
            phoneHidden={compact && item.id !== items.find((i) => !i.done)?.id}
            onMarkDone={item.id === 'phone' ? () => setPrefs((p) => ({ ...p, phoneDone: true })) : undefined}
          />
        ))}
      </ul>
    </section>
  )
}
