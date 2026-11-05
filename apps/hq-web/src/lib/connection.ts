import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { fetchHello, type Hello, helloUpstreamGone, startingSeconds } from '~/lib/hello'
import { UNREACHABLE_GRACE_MS, useDeviceOnline } from '~/lib/reachability'
import { hqJson, needsSignIn, onSignInChange } from '~/lib/hqAuth'
import { useHQStore } from '~/store/hqStore'
import { t } from '~/strings'

export type ConnectionState =
  | 'signed_out'
  | 'offline'
  | 'unreachable'
  | 'starting'
  | 'reconnecting'
  | 'ai_down'
  | 'connected'

export type ConnectionDot = 'warn' | 'work' | 'off'

export interface ConnectionInputs {
  status401: boolean
  online: boolean
  helloAnsweredAt: number | null
  lastHelloMs: number | null
  hello: Hello | null
  socketOpen: boolean
  aiDown: boolean
  proxyUpstreamGone: boolean
  lite: boolean
  inHub: boolean
}

export interface ConnectionView {
  state: ConnectionState
  words: string
  dot: ConnectionDot
  actionsOff: string | null
  hello: Hello | null
  online: boolean
  lastHelloAnsweredAt: number | null
  nextTryMs: number | null
}

export interface UseConnectionResult extends ConnectionView {
  tryNow: () => void
}

export const RETRY_SCHEDULE_SEC = [5, 10, 20] as const
export const CONNECTED_POLL_MS = 20_000
/** How often a signed-in page asks whether HQ's AI services answer (`/api/status/ai`). */
export const AI_STATUS_POLL_MS = 60_000
const AI_STATUS_URL = '/api/status/ai'

// Gives the retry delay in seconds for unreachable backoff.
export function retryDelaySec(attemptIndex: number): number {
  if (attemptIndex <= 0) return 5
  if (attemptIndex === 1) return 10
  return 20
}

export function retryDelayMs(attemptIndex: number): number {
  return retryDelaySec(attemptIndex) * 1000
}

function isUnreachable(inputs: ConnectionInputs, nowMs: number): boolean {
  if (inputs.proxyUpstreamGone) return true
  if (inputs.hello !== null) return false
  if (inputs.helloAnsweredAt !== null) {
    return nowMs - inputs.helloAnsweredAt >= UNREACHABLE_GRACE_MS
  }
  if (inputs.lastHelloMs !== null) {
    return nowMs - inputs.lastHelloMs >= UNREACHABLE_GRACE_MS
  }
  // Not asked yet (the first paint, the prerender): nothing says HQ is out of reach.
  return false
}

// First true row wins (rules-web section 2).
export function connectionState(
  inputs: ConnectionInputs,
  nowMs: number,
  nextTryMs: number | null = null,
): ConnectionView {
  const base = {
    hello: inputs.hello,
    online: inputs.online,
    lastHelloAnsweredAt: inputs.helloAnsweredAt,
    nextTryMs,
  }

  if (inputs.status401) {
    return {
      ...base,
      state: 'signed_out',
      words: t('state_signed_out'),
      dot: 'warn',
      actionsOff: null,
    }
  }

  if (!inputs.online) {
    return {
      ...base,
      state: 'offline',
      words: t('state_offline'),
      dot: 'warn',
      actionsOff: null,
    }
  }

  if (isUnreachable(inputs, nowMs)) {
    return {
      ...base,
      state: 'unreachable',
      words: t('state_unreachable'),
      dot: 'warn',
      actionsOff: t('off_until_reached'),
    }
  }

  if (inputs.hello?.starting != null) {
    const seconds = startingSeconds(inputs.hello.starting, nowMs)
    return {
      ...base,
      state: 'starting',
      words: t('state_starting', { seconds }),
      dot: 'work',
      actionsOff: t('off_until_started'),
    }
  }

  if (!inputs.socketOpen) {
    return {
      ...base,
      state: 'reconnecting',
      words: t('state_reconnecting'),
      dot: 'work',
      actionsOff: null,
    }
  }

  if (inputs.aiDown) {
    return {
      ...base,
      state: 'ai_down',
      words: t('state_ai_down'),
      dot: 'warn',
      actionsOff: null,
    }
  }

  return {
    ...base,
    state: 'connected',
    words: inputs.lite ? t('state_lite') : t('state_connected'),
    dot: 'off',
    actionsOff: null,
  }
}

export interface UseConnectionOptions {
  aiDown?: boolean
  proxyUpstreamGone?: boolean
  inHub?: boolean
}

/** Whether HQ says its AI services are not answering; asked only while `enabled` and the tab shows. */
function useAiDown(enabled: boolean): boolean {
  const [down, setDown] = useState(false)
  useEffect(() => {
    if (!enabled) {
      setDown(false)
      return undefined
    }
    let alive = true
    const ask = () => {
      if (typeof document !== 'undefined' && document.hidden) return
      hqJson<{ answering: boolean }>(AI_STATUS_URL)
        .then((r) => alive && setDown(!r.answering))
        .catch(() => undefined)
    }
    ask()
    const id = window.setInterval(ask, AI_STATUS_POLL_MS)
    return () => {
      alive = false
      window.clearInterval(id)
    }
  }, [enabled])
  return down
}

export function useConnection(options?: UseConnectionOptions): UseConnectionResult {
  const online = useDeviceOnline()
  const socketOpen = useHQStore((s) => s.wsConnected)
  const status401 = useSyncExternalStore(onSignInChange, needsSignIn, () => false)

  const [hello, setHello] = useState<Hello | null>(null)
  const [helloAnsweredAt, setHelloAnsweredAt] = useState<number | null>(null)
  const [lastHelloMs, setLastHelloMs] = useState<number | null>(null)
  const [nextTryMs, setNextTryMs] = useState<number | null>(null)
  const [retryAttempt, setRetryAttempt] = useState(0)
  const [nowMs, setNowMs] = useState(() => Date.now())

  const wasUnreachableRef = useRef(false)
  const retryAttemptRef = useRef(retryAttempt)
  retryAttemptRef.current = retryAttempt
  const timerRef = useRef<number | null>(null)
  const abortControllerRef = useRef<AbortController | null>(null)

  const lite = hello?.edition === 'lite'
  const polledAiDown = useAiDown(!status401 && hello !== null && hello.edition !== 'lite')
  const aiDown = options?.aiDown ?? polledAiDown
  const proxyUpstreamGone = options?.proxyUpstreamGone ?? helloUpstreamGone()
  const inHub = options?.inHub ?? false

  const inputs: ConnectionInputs = {
    status401,
    online,
    helloAnsweredAt,
    lastHelloMs,
    hello,
    socketOpen,
    aiDown,
    proxyUpstreamGone,
    lite,
    inHub,
  }

  const view = connectionState(inputs, nowMs, nextTryMs)

  // Shows the back toast when HQ recovers after unreachable.
  useEffect(() => {
    if (view.state === 'unreachable') {
      wasUnreachableRef.current = true
    } else if (wasUnreachableRef.current && (view.state === 'connected' || view.state === 'starting')) {
      wasUnreachableRef.current = false
      useHQStore.getState().setSystemNotice(t('back'))
    }
  }, [view.state])

  // Keeps seconds accurate for starting and next-try counters.
  useEffect(() => {
    if (view.state === 'starting' || view.state === 'unreachable') {
      const id = window.setInterval(() => setNowMs(Date.now()), 1000)
      return () => window.clearInterval(id)
    }
    return undefined
  }, [view.state])

  const clearScheduled = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
    setNextTryMs(null)
  }, [])

  const poll = useCallback(async () => {
    clearScheduled()
    abortControllerRef.current?.abort()
    const ac = new AbortController()
    abortControllerRef.current = ac

    const attemptStart = Date.now()
    setLastHelloMs(attemptStart)
    setNowMs(attemptStart)

    const result = await fetchHello(ac.signal)
    if (ac.signal.aborted) return

    const finishedAt = Date.now()
    setNowMs(finishedAt)

    if (result) {
      setHello(result)
      setHelloAnsweredAt(finishedAt)
      setRetryAttempt(0)
      scheduleNext(CONNECTED_POLL_MS)
    } else {
      setHello(null)
      const nextAttempt = retryAttemptRef.current + 1
      setRetryAttempt(nextAttempt)
      const delay = retryDelayMs(retryAttemptRef.current)
      scheduleNext(delay)
    }
  }, [clearScheduled])

  const scheduleNext = (delayMs: number) => {
    if (typeof document !== 'undefined' && document.hidden) {
      setNextTryMs(null)
      return
    }
    const target = Date.now() + delayMs
    setNextTryMs(target)
    if (timerRef.current !== null) window.clearTimeout(timerRef.current)
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null
      void poll()
    }, delayMs)
  }

  const tryNow = useCallback(() => {
    void poll()
  }, [poll])

  // Initial poll and cleanup.
  useEffect(() => {
    void poll()
    return () => {
      clearScheduled()
      abortControllerRef.current?.abort()
    }
  }, [poll, clearScheduled])

  // Pauses while tab is hidden, tries once when visible.
  useEffect(() => {
    const onVisibilityChange = () => {
      if (document.hidden) {
        clearScheduled()
      } else {
        void poll()
      }
    }
    document.addEventListener('visibilitychange', onVisibilityChange)
    return () => document.removeEventListener('visibilitychange', onVisibilityChange)
  }, [poll, clearScheduled])

  // Retries when online event fires.
  useEffect(() => {
    const onOnline = () => {
      void poll()
    }
    window.addEventListener('online', onOnline)
    return () => window.removeEventListener('online', onOnline)
  }, [poll])

  return {
    ...view,
    tryNow,
  }
}
