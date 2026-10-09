// Pure helpers for the Workbench live screen: the SSE parser and the reconnect and mode decisions.

export interface SseEvent {
  event: string
  data: string
}

/** Incremental SSE parser. Feed raw text chunks; complete events come back as they finish. */
export function createSseParser() {
  let buffer = ''
  let event = ''
  let data: string[] = []

  const line = (l: string, out: SseEvent[]) => {
    if (l === '') {
      if (data.length > 0) out.push({ event: event || 'message', data: data.join('\n') })
      event = ''
      data = []
      return
    }
    if (l.startsWith(':')) return
    const colon = l.indexOf(':')
    const field = colon === -1 ? l : l.slice(0, colon)
    let value = colon === -1 ? '' : l.slice(colon + 1)
    if (value.startsWith(' ')) value = value.slice(1)
    if (field === 'event') event = value
    else if (field === 'data') data.push(value)
  }

  return {
    feed(chunk: string): SseEvent[] {
      buffer += chunk
      const out: SseEvent[] = []
      for (;;) {
        const m = /\r\n|\r|\n/.exec(buffer)
        if (!m) break
        // A lone CR at the very end may be the first half of CRLF; wait for more.
        if (m[0] === '\r' && m.index === buffer.length - 1) break
        line(buffer.slice(0, m.index), out)
        buffer = buffer.slice(m.index + m[0].length)
      }
      return out
    },
  }
}

export const MAX_STREAM_RETRIES = 3
const RECONNECT_BASE_MS = 1_000

/** Delay before reconnect attempt number `failures`, or null once the stream should give way to polling. */
export function reconnectDelay(failures: number): number | null {
  if (failures > MAX_STREAM_RETRIES) return null
  return RECONNECT_BASE_MS * 2 ** (failures - 1)
}

export type StreamPhase = 'connecting' | 'live' | 'failed' | 'ended'

/** Stream while it can work; poll once it failed for good or the session ended (to fetch the saved view). */
export function chooseScreenMode(phase: StreamPhase): 'stream' | 'poll' {
  return phase === 'failed' || phase === 'ended' ? 'poll' : 'stream'
}

export const HIDDEN_CLOSE_MS = 30_000

export function screenStatusText(mode: 'stream' | 'poll', phase: StreamPhase, source?: 'live' | 'snapshot'): string {
  if (source === 'snapshot') return 'Last saved view'
  if (mode === 'poll') return 'Updating every few seconds'
  return phase === 'live' ? 'Live' : 'Connecting'
}
