import { useCallback, useEffect, useRef, useState } from 'react'

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
  everyMs: number
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
    timer = setTimeout(tick, backoffDelay(everyMs, failures))
  }

  const tick = () => {
    if (isVisible()) void run()
    else schedule()
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
export function usePolled<T>(key: string, load: () => Promise<T>, everyMs: number, enabled = true): Polled<T> {
  const [state, setState] = useState<{ key: string; data: T | null; error: string | null } | null>(null)
  const loadRef = useRef(load)
  loadRef.current = load
  const pollerRef = useRef<ReturnType<typeof createPoller<T>> | null>(null)

  useEffect(() => {
    if (!enabled) return
    const poller = createPoller<T>({
      load: () => loadRef.current(),
      onData: (data) => setState({ key, data, error: null }),
      // Keep the last good data on screen; a failed refresh only adds the error.
      onError: (error) => setState((prev) => ({ key, data: prev?.key === key ? prev.data : null, error })),
      everyMs,
      isVisible: () => document.visibilityState === 'visible',
    })
    pollerRef.current = poller
    poller.start()
    // Polls skip while the tab is hidden, so catch up the moment it is shown again.
    const onVisible = () => {
      if (document.visibilityState === 'visible') void poller.refresh()
    }
    document.addEventListener('visibilitychange', onVisible)
    return () => {
      document.removeEventListener('visibilitychange', onVisible)
      poller.stop()
      if (pollerRef.current === poller) pollerRef.current = null
    }
  }, [key, enabled, everyMs])

  const refresh = useCallback(() => pollerRef.current?.refresh() ?? Promise.resolve(), [])
  const current = state?.key === key ? state : null
  return { data: current?.data ?? null, error: current?.error ?? null, loading: enabled && current === null, refresh }
}
