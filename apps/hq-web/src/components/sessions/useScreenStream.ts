import { useEffect, useState } from 'react'
import { HqHttpError, hqFetch } from '~/lib/hqAuth'
import type { ScreenText } from '~/lib/sessionsApi'
import { HIDDEN_CLOSE_MS, createSseParser, reconnectDelay, type StreamPhase } from '~/lib/screenStream'

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

/** Reads one connection to the end. Returns how it ended; `onScreen` fires for each screen event. */
async function readStream(url: string, signal: AbortSignal, onScreen: (s: ScreenText) => void): Promise<'ended' | 'closed'> {
  const res = await hqFetch(url, { signal, headers: { Accept: 'text/event-stream' } })
  if (!res.ok || !res.body) throw new HqHttpError(`${res.status} ${res.statusText}`, res.status)
  const reader = res.body.getReader()
  const decoder = new TextDecoder()
  const parser = createSseParser()
  for (;;) {
    const { done, value } = await reader.read()
    if (done) return 'closed'
    for (const ev of parser.feed(decoder.decode(value, { stream: true }))) {
      if (ev.event === 'end') return 'ended'
      if (ev.event !== 'screen') continue
      try {
        onScreen(JSON.parse(ev.data) as ScreenText)
      } catch {
        // A malformed event is skipped; the next one replaces it.
      }
    }
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
        try {
          const how = await readStream(url, signal, (data) => {
            failures = 0
            set({ data, phase: 'live' })
          })
          if (signal.aborted) return
          if (how === 'ended') return set({ phase: 'ended' })
        } catch {
          if (signal.aborted) return
        }
        failures += 1
        const delay = reconnectDelay(failures)
        if (delay === null) return set({ phase: 'failed' })
        set({ phase: 'connecting' })
        await sleep(delay, signal)
      }
    })()
    return () => ctrl.abort()
  }, [sessionId, url, enabled, hiddenLong])

  const current = state?.key === sessionId ? state : null
  return { data: current?.data ?? null, phase: current?.phase ?? 'connecting' }
}
