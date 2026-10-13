import { useEffect, useState, useSyncExternalStore } from 'react'
import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { Check, Circle } from 'lucide-react'
import { CHAT_APPS_QUERY_KEY, fetchChatApps } from '~/lib/chatAppsApi'
import { needsSignIn, onSignInChange } from '~/lib/hqAuth'
import { getPushState, isStandalone, type PushState } from '~/lib/webPush'
import { usePersistedState } from '~/lib/usePersistedState'
import {
  DEFAULT_GET_STARTED_PREFS,
  GET_STARTED_KEY,
  getStartedItems,
  getStartedVisible,
  isGetStartedPrefs,
  serverUnreachable,
  type GetStartedItem,
} from '~/lib/getStarted'

// A service worker that never becomes ready would otherwise keep the card from ever settling.
const PUSH_CHECK_TIMEOUT_MS = 5000
const TAP_TARGET_PX = 36

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

function ItemRow({ item, onMarkDone }: { item: GetStartedItem; onMarkDone?: () => void }) {
  const Icon = item.done ? Check : Circle
  return (
    <li className="flex items-start gap-3 py-3 border-t border-white/5 first:border-t-0">
      <Icon
        className="w-4 h-4 mt-0.5 shrink-0"
        style={{ color: item.done ? 'var(--accent-green)' : 'var(--text-dim)' }}
        aria-hidden="true"
      />
      <div className="flex flex-col gap-1 min-w-0 flex-1">
        <span className={`text-sm font-bold ${item.done ? 'line-through' : ''}`} style={{ color: item.done ? 'var(--text-dim)' : 'var(--text-primary)' }}>
          {item.title}
          <span className="sr-only">{item.done ? ' (done)' : ' (to do)'}</span>
        </span>
        {!item.done && (
          <>
            <p className="text-xs leading-relaxed" style={{ color: 'var(--text-dim)' }}>{item.detail}</p>
            <div className="flex flex-wrap items-center gap-2 pt-1">
              <Link
                to={item.to}
                hash={item.hash}
                className="inline-flex items-center px-3 rounded-lg bg-white/5 text-xs font-bold"
                style={{ color: 'var(--text-primary)', minHeight: TAP_TARGET_PX }}
              >
                {item.linkLabel}
              </Link>
              {onMarkDone && (
                <button
                  type="button"
                  onClick={onMarkDone}
                  className="px-3 rounded-lg text-xs"
                  style={{ color: 'var(--text-dim)', minHeight: TAP_TARGET_PX }}
                >
                  Mark done
                </button>
              )}
            </div>
          </>
        )}
      </div>
    </li>
  )
}

/** First-run checklist on the home screen: the things an owner would not otherwise find. */
export function GetStartedCard() {
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
        <h2 id="get-started-title" className="text-[11px] tracking-widest uppercase font-bold" style={{ color: 'var(--text-dim)' }}>
          Get started
        </h2>
        <div className="flex-1 h-px bg-white/5" />
        <button
          type="button"
          onClick={() => setPrefs((p) => ({ ...p, hidden: true }))}
          className="px-3 rounded-lg bg-white/5 text-[11px] font-bold"
          style={{ color: 'var(--text-primary)', minHeight: TAP_TARGET_PX }}
        >
          Hide
        </button>
      </div>
      <ul className="glass-card rounded-xl px-4">
        {items.map((item) => (
          <ItemRow
            key={item.id}
            item={item}
            onMarkDone={item.id === 'phone' ? () => setPrefs((p) => ({ ...p, phoneDone: true })) : undefined}
          />
        ))}
      </ul>
    </section>
  )
}
