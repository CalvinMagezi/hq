import { useCallback, useEffect, useRef, useState } from 'react'
import { useWSOptional } from '~/context/WebSocketContext'

interface Polled<T> {
  data: T | null
  error: string | null
  /** True until the first response (or failure) arrives for the current key. */
  loading: boolean
  refresh: () => Promise<void>
}

/** Ceiling for the delay between polls while requests keep failing. */
export const MAX_BACKOFF_MS = 30_000
const MAX_BACKOFF_DOUBLINGS = 10

/** Delay before the next poll: the normal period, doubled per consecutive failure. */
export function backoffDelay(everyMs: number, failures: number): number {
  return Math.min(everyMs * 2 ** Math.min(failures, MAX_BACKOFF_DOUBLINGS), Math.max(MAX_BACKOFF_MS, everyMs))
}

interface PollerOptions<T> {
  load: () => Promise<T>
  onData: (data: T) => void
  onError: (message: string) => void
  /** A function lets the period change (fast to slow) without restarting the poller. 0 means no timer at all. */
  everyMs: number | (() => number)
  isVisible?: () => boolean
}

/**
 * One request at a time: the next poll is scheduled only after the previous
 * one settles, so a slow host can never stack requests. A refresh asked for
 * mid-flight runs once more right after it, so a send is never shown stale.
 */
export function createPoller<T>({ load, onData, onError, everyMs, isVisible = () => true }: PollerOptions<T>) {
  let stopped = false
  let failures = 0
  let queued = false
  let current: Promise<void> | null = null
  let timer: ReturnType<typeof setTimeout> | undefined

  const schedule = () => {
    clearTimeout(timer)
    if (stopped) return
    const period = typeof everyMs === 'function' ? everyMs() : everyMs
    if (period <= 0) return
    timer = setTimeout(tick, backoffDelay(period, failures))
  }

  // A hidden tab keeps no timer at all; `wake` starts it again when the tab is shown.
  const tick = () => {
    if (isVisible()) void run()
  }

  const run = (): Promise<void> => {
    if (stopped) return Promise.resolve()
    if (current) {
      queued = true
      return current
    }
    clearTimeout(timer)
    current = (async () => {
      try {
        // Deferred so a load that throws synchronously still settles after `current` is set.
        const data = await Promise.resolve().then(load)
        failures = 0
        if (!stopped) onData(data)
      } catch (err) {
        failures += 1
        if (!stopped) onError(err instanceof Error ? err.message : 'Request failed')
      } finally {
        current = null
      }
      if (queued && !stopped) {
        queued = false
        await run()
        return
      }
      schedule()
    })()
    return current
  }

  return {
    start: () => void run(),
    refresh: run,
    // A tab returning to view loads once at once and starts the timer again.
    wake: () => void run(),
    stop: () => {
      stopped = true
      clearTimeout(timer)
    },
  }
}

/**
 * Loads now and then repeatedly while the tab is visible. `key` identifies
 * what is loaded: when it changes the old data is dropped and the old poller
 * stopped, so a slow response for the previous key never shows under the new one.
 */
export function usePolled<T>(
  key: string,
  load: () => Promise<T>,
  everyMs: number,
  enabled = true,
  _refreshOnVisible = true,
  /** Socket events that reload this; with any, there is no timer (`everyMs` is then only the old fallback and is ignored). */
  events?: readonly string[],
): Polled<T> {
  const [state, setState] = useState<{ key: string; data: T | null; error: string | null } | null>(null)
  const loadRef = useRef(load)
  loadRef.current = load
  const everyRef = useRef(everyMs)
  everyRef.current = events?.length ? 0 : everyMs
  const pollerRef = useRef<ReturnType<typeof createPoller<T>> | null>(null)

  useEffect(() => {
    if (!enabled) return
    const poller = createPoller<T>({
      load: () => loadRef.current(),
      onData: (data) => setState({ key, data, error: null }),
      // Keep the last good data on screen; a failed refresh only adds the error.
      onError: (error) => setState((prev) => ({ key, data: prev?.key === key ? prev.data : null, error })),
      everyMs: () => everyRef.current,
      isVisible: () => document.visibilityState === 'visible',
    })
    pollerRef.current = poller
    poller.start()
    // Polls skip while the tab is hidden, so catch up the moment it is shown again.
    const onVisible = () => {
      if (document.visibilityState === 'visible') poller.wake()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      document.removeEventListener('visibilitychange', onVisible)
      poller.stop()
      if (pollerRef.current === poller) pollerRef.current = null
    }
  }, [key, enabled])

  const eventKey = events?.join('|') ?? ''
  const ws = useWSOptional()
  const subscribe = ws?.subscribe
  const connected = ws?.connected
  useEffect(() => {
    if (!enabled || !eventKey || !subscribe) return
    const wanted = eventKey.split('|')
    return subscribe((msg) => {
      if (wanted.includes(String(msg.type)) && document.visibilityState === 'visible') void pollerRef.current?.refresh()
    })
  }, [enabled, eventKey, subscribe])
  // Events sent while the socket was down are gone: a reconnect loads once.
  useEffect(() => {
    if (enabled && eventKey && connected) void pollerRef.current?.refresh()
  }, [enabled, eventKey, connected])

  const refresh = useCallback(() => pollerRef.current?.refresh() ?? Promise.resolve(), [])
  const current = state?.key === key ? state : null
  return { data: current?.data ?? null, error: current?.error ?? null, loading: enabled && current === null, refresh }
}
