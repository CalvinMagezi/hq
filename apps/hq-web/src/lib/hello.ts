// `GET /api/hello`: which HQ answers, and whether it is still starting (crates/hq-web/src/hello.rs).
// Open without a sign-in, so it also tells a signed-out browser which HQ's saved copy is its own.

import { noteHqId } from './savedCopy'

export interface StartStep {
  /** 1-based. */
  step: number
  steps: number
  step_name: string
  /** Unix seconds. */
  started_at: number
}

export interface Hello {
  hq_id: string
  edition: 'full' | 'lite' | 'mobile' | string
  version: string
  min_client: number
  /** HQ's own IANA zone, or null when the host will not say. */
  tz?: string | null
  tz_offset_minutes?: number
  starting: StartStep | null
}

export const HELLO_URL = '/api/hello'
/** HQ Hub's proxy marks its own 502 when the HQ behind it is gone (crates/hq-desktop/src/proxy.rs). */
const UPSTREAM_GONE = { header: 'x-hq-proxy', value: 'upstream-gone' }
/** The same 502 when the person stopped HQ from HQ Hub on purpose. */
const YOU_STOPPED = 'stopped'

let upstreamGone = false
let youStopped = false

/** Whether the last hello came back from HQ Hub saying the HQ behind it is gone. */
export function helloUpstreamGone(): boolean {
  return upstreamGone
}

/** Whether the last hello came back from HQ Hub saying the person stopped HQ themselves. */
export function helloYouStopped(): boolean {
  return youStopped
}

export function isHello(v: unknown): v is Hello {
  const h = v as Partial<Hello> | null
  return !!h && typeof h.hq_id === 'string' && h.hq_id.length > 0 && typeof h.version === 'string'
}

/** One hello. Resolves null when HQ did not answer with one (down, a proxy page, a timeout). */
export async function fetchHello(signal?: AbortSignal): Promise<Hello | null> {
  try {
    const res = await fetch(HELLO_URL, { signal, cache: 'no-store' })
    const mark = res.headers.get(UPSTREAM_GONE.header)
    youStopped = mark === YOU_STOPPED
    upstreamGone = mark === UPSTREAM_GONE.value || youStopped
    if (!res.ok) return null
    const body: unknown = await res.json()
    if (!isHello(body)) return null
    // Every hello tells the saved copy which HQ it belongs to; a different HQ reloads the page.
    noteHqId(body.hq_id)
    return body
  } catch {
    return null
  }
}

/** Seconds HQ has been starting, from its own clock reading; never negative. */
export function startingSeconds(step: StartStep, nowMs: number): number {
  return Math.max(0, Math.floor(nowMs / 1000 - step.started_at))
}
