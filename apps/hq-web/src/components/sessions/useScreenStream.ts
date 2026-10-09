import { useEffect, useState } from 'react'
import { HqHttpError, hqFetch } from '~/lib/hqAuth'
import type { ScreenText } from '~/lib/sessionsApi'
import {
  FIRST_EVENT_TIMEOUT_MS,
  HIDDEN_CLOSE_MS,
  createSseParser,
  decideOnEnd,
  failuresAfterDrop,
  parseEndReason,
  reconnectDelay,
  type EndReason,
  type StreamPhase,
} from '~/lib/screenStream'

interface StreamState {
  key: string
  data: ScreenText | null
  phase: StreamPhase
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const t = setTimeout(resolve, ms)
    signal.addEventListener('abort', () => (clearTimeout(t), resolve()), { once: true })
  })
}

type Outcome = { kind: 'end'; reason: EndReason } | { kind: 'closed' }

/** Reads one connection to the end. Aborts itself if no first event arrives in time, which surfaces as a throw. */
async function readStream(url: string, outer: AbortSignal, onScreen: (s: ScreenText) => void): Promise<Outcome> {
  const conn = new AbortController()
  const abortConn = () => conn.abort()
  outer.addEventListener('abort', abortConn, { once: true })
  let firstEventTimer: ReturnType<typeof setTimeout> | undefined = setTimeout(abortConn, FIRST_EVENT_TIMEOUT_MS)
  const clearTimer = () => {
    clearTimeout(firstEventTimer)
    firstEventTimer = undefined
  }
  try {
    const res = await hqFetch(url, { signal: conn.signal, headers: { Accept: 'text/event-stream' } })
    if (!res.ok || !res.body) throw new HqHttpError(`${res.status} ${res.statusText}`, res.status)
    const reader = res.body.getReader()
    const decoder = new TextDecoder()
    const parser = createSseParser()
    for (;;) {
      const { done, value } = await reader.read()
      if (done) return { kind: 'closed' }
      for (const ev of parser.feed(decoder.decode(value, { stream: true }))) {
        clearTimer()
        if (ev.event === 'end') return { kind: 'end', reason: parseEndReason(ev.data) }
        if (ev.event !== 'screen') continue
        try {
          onScreen(JSON.parse(ev.data) as ScreenText)
        } catch {
          // A malformed event is skipped; the next one replaces it.
        }
      }
    }
  } finally {
    clearTimer()
    outer.removeEventListener('abort', abortConn)
    conn.abort()
  }
}

/**
 * Live screen over server-sent events. Reconnects with backoff, then reports phase
 * 'failed' so the caller can fall back to polling. Closes after the tab has been
 * hidden for HIDDEN_CLOSE_MS and reopens when it is shown again.
 */
export function useScreenStream(sessionId: string, url: string, enabled: boolean): { data: ScreenText | null; phase: StreamPhase } {
  const [state, setState] = useState<StreamState | null>(null)
  const [hiddenLong, setHiddenLong] = useState(false)

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined
    const onChange = () => {
      clearTimeout(timer)
      if (document.visibilityState === 'visible') setHiddenLong(false)
      else timer = setTimeout(() => setHiddenLong(true), HIDDEN_CLOSE_MS)
    }
    document.addEventListener('visibilitychange', onChange)
    return () => {
      clearTimeout(timer)
      document.removeEventListener('visibilitychange', onChange)
    }
  }, [])

  useEffect(() => {
    if (!enabled || hiddenLong) return
    const ctrl = new AbortController()
    const { signal } = ctrl
    const set = (patch: Partial<StreamState>) =>
      setState((prev) => ({ ...(prev?.key === sessionId ? prev : { key: sessionId, data: null, phase: 'connecting' as const }), ...patch }))
    void (async () => {
      let failures = 0
      while (!signal.aborted) {
        const startedAt = Date.now()
        let events = 0
        let outcome: Outcome | null = null
        try {
          outcome = await readStream(url, signal, (data) => {
            events += 1
            set({ data, phase: 'live' })
          })
        } catch {
          // Network error, non-2xx or first-event timeout: handled as a dropped connection below.
        }
        if (signal.aborted) return
        let delay: number | null
        if (outcome?.kind === 'end') {
          const decision = decideOnEnd(outcome.reason, failures)
          if (decision.action === 'poll') return set({ phase: 'ended' })
          if (decision.action === 'fail') return set({ phase: 'failed' })
          failures = decision.failures
          delay = decision.delay
        } else {
          failures = failuresAfterDrop(failures, Date.now() - startedAt, events)
          delay = reconnectDelay(failures)
          if (delay === null) return set({ phase: 'failed' })
        }
        if (delay > 0) {
          set({ phase: 'connecting' })
          await sleep(delay, signal)
        }
      }
    })()
    return () => ctrl.abort()
  }, [sessionId, url, enabled, hiddenLong])

  const current = state?.key === sessionId ? state : null
  return { data: current?.data ?? null, phase: current?.phase ?? 'connecting' }
}
